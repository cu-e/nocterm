//! The sidebar list of saved connections.

use std::{collections::HashSet, sync::Arc};

use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, Hsla, Image, ImageFormat,
    MouseButton, ObjectFit, SharedString, StyledImage as _, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div, img,
    prelude::*,
};
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use crate::{
    Connections, ServerFacts,
    model::{connect, spec_for_profile},
    open_editor,
    os::{self, Os},
    store::{Profile, ProfileId},
};

/// A server's icon: `os` in `color` (its brand colour when none), or the
/// generic server icon in `color` (`fallback` when none).
pub(crate) fn server_icon(os: Option<&Os>, color: Option<&str>, fallback: Hsla) -> AnyElement {
    match os {
        Some(os) => img(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            os::svg(os, color.unwrap_or(os.color)),
        )))
        .size_4()
        .flex_shrink_0()
        .into_any_element(),
        None => Icon::new(IconName::Server)
            .small()
            .text_color(
                color
                    .and_then(os::rgb)
                    .map_or(fallback, |rgb| gpui_kit::rgb(rgb).into()),
            )
            .into_any_element(),
    }
}

/// Shared by every row, so hovering a row reveals only its own buttons.
const ROW_GROUP: &str = "connection-row";
/// Same for group headers.
const GROUP_HEADER: &str = "connection-group-header";

/// A group name being edited in place.
struct GroupRename {
    group: String,
    input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy)]
enum GroupAction {
    Ungroup,
    Delete,
}

#[derive(Clone)]
struct DraggedProfile(ProfileId);

fn run_group_action(
    action: GroupAction,
    group: String,
    expected: Vec<ProfileId>,
    panel: WeakEntity<ConnectionsPanel>,
    window: &mut Window,
    cx: &mut App,
) {
    let (done, key, title) = Connections::global(cx).update(cx, |connections, cx| match action {
        GroupAction::Ungroup => (
            connections.ungroup(group.clone(), cx),
            "connections-ungroup",
            "Could not ungroup connections",
        ),
        GroupAction::Delete => (
            connections.delete_group(group.clone(), expected, cx),
            "connections-delete-group",
            "Could not delete group",
        ),
    });
    window
        .spawn(cx, async move |cx| match done.await {
            Ok(()) => {
                let _ = panel.update(cx, |panel, cx| {
                    panel.collapsed.remove(&group);
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = cx
                    .update(|window, cx| nocterm_ui::notice::error(window, cx, key, title, error));
            }
        })
        .detach();
}

fn description_preview(description: &str) -> String {
    description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}
fn description_tooltip(description: &str) -> String {
    let mut chars = description.chars();
    let text: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}

pub struct ConnectionsPanel {
    connections: Entity<Connections>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    filter: Entity<InputState>,
    /// Folders the user folded away.
    collapsed: HashSet<String>,
    renaming: Option<GroupRename>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionsPanel {
    pub fn new(
        connections: Entity<Connections>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let subscriptions = vec![
            cx.observe_in(&connections, window, |this, connections, window, cx| {
                // The group may have been deleted or ungrouped while being renamed.
                if let Some(rename) = &this.renaming
                    && !connections.read(cx).profiles().has_group(&rename.group)
                {
                    let focused = rename.input.read(cx).focus_handle(cx).is_focused(window);
                    this.renaming = None;
                    if focused {
                        window.focus(&this.focus_handle, cx);
                    }
                }
                cx.notify();
            }),
            // Detected systems and countries, and flags as they load.
            cx.observe(&ServerFacts::global(cx), |_, _, cx| cx.notify()),
            // Turning country detection on or off shows or hides detected flags.
            cx.observe_global::<nocterm_ui::SettingsStore>(|_, cx| cx.notify()),
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                // Enter opens the only match.
                InputEvent::PressEnter { .. } => this.open_single_match(window, cx),
                _ => {}
            }),
        ];
        Self {
            connections,
            workspace,
            focus_handle: cx.focus_handle(),
            filter,
            collapsed: HashSet::new(),
            renaming: None,
            _subscriptions: subscriptions,
        }
    }

    fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    fn open(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.connections.read(cx).profiles().get(id).cloned() else {
            return;
        };
        connect(
            &self.workspace,
            spec_for_profile(&profile),
            Some(id),
            window,
            cx,
        );
    }

    fn open_single_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter_text(cx);
        let matches: Vec<ProfileId> = self
            .connections
            .read(cx)
            .profiles()
            .iter()
            .filter(|profile| profile.matches(&filter))
            .map(|profile| profile.id)
            .collect();
        if let [id] = matches[..] {
            self.open(id, window, cx);
        }
    }

    fn edit(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.connections.read(cx).profiles().get(id).cloned();
        if profile.is_some() {
            open_editor(profile, self.workspace.clone(), window, cx);
        }
    }

    fn confirm_delete(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self
            .connections
            .read(cx)
            .profiles()
            .get(id)
            .map(|profile| profile.name.clone())
        else {
            return;
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title(format!("Delete “{name}”?"))
                .description("The saved connection is removed. Open sessions stay open.")
                .ok_text("Delete")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let deleted = Connections::global(cx)
                        .update(cx, |connections, cx| connections.delete_profile(id, cx));
                    window
                        .spawn(cx, async move |cx| {
                            if let Err(error) = deleted.await {
                                let _ = cx.update(|window, cx| {
                                    nocterm_ui::notice::error(
                                        window,
                                        cx,
                                        "connections-delete",
                                        "Could not delete connection",
                                        error,
                                    )
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }

    fn confirm_delete_group(&mut self, group: String, window: &mut Window, cx: &mut Context<Self>) {
        // The delete only applies while the group still holds these connections.
        let members: Vec<ProfileId> = self
            .connections
            .read(cx)
            .profiles()
            .in_group(Some(&group), "")
            .iter()
            .map(|profile| profile.id)
            .collect();
        let count = members.len();
        let panel = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let description = match count {
                0 => "This group is empty.".to_owned(),
                1 => "1 connection in this group. Delete it, or ungroup to move it to Ungrouped. Open sessions stay open. Saved vault credentials are kept.".to_owned(),
                n => format!("{n} connections in this group. Delete them, or ungroup to move them to Ungrouped. Open sessions stay open. Saved vault credentials are kept."),
            };
            let action = |id: &'static str, label: &'static str, action: GroupAction| {
                let (group, panel, members) = (group.clone(), panel.clone(), members.clone());
                Button::new(id)
                    .label(label)
                    .when(matches!(action, GroupAction::Delete), |button| button.danger())
                    .on_click(move |_: &ClickEvent, window, cx| {
                        run_group_action(
                            action,
                            group.clone(),
                            members.clone(),
                            panel.clone(),
                            window,
                            cx,
                        );
                        window.close_dialog(cx);
                    })
            };
            alert
                .title(format!("Delete group “{group}”?"))
                .description(description)
                // Enter must neither delete nor dismiss: only the buttons act.
                .on_ok(|_, _, _| false)
                .footer(
                    DialogFooter::new()
                        .child(Button::new("group-delete-cancel").label("Cancel").on_click(
                            |_: &ClickEvent, window, cx| window.close_dialog(cx),
                        ))
                        .when(count > 0, |footer| {
                            footer.child(action(
                                "group-delete-ungroup",
                                "Ungroup",
                                GroupAction::Ungroup,
                            ))
                        })
                        .child(action(
                            "group-delete-all",
                            if count > 0 {
                                "Delete group and connections"
                            } else {
                                "Delete group"
                            },
                            GroupAction::Delete,
                        )),
                )
        });
    }

    fn start_rename(&mut self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.as_ref().is_some_and(|r| r.group == group) {
            return;
        }
        self.finish_rename(true, false, window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(group.to_owned()));
        let focus = input.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.finish_rename(true, true, window, cx);
                }
            }),
            cx.on_blur(&focus, window, |this, window, cx| {
                this.finish_rename(true, false, window, cx)
            }),
        ];
        self.renaming = Some(GroupRename {
            group: group.to_owned(),
            input,
            _subscriptions: subscriptions,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn finish_rename(
        &mut self,
        accept: bool,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Dropping the state first also drops the blur subscription.
        let Some(GroupRename { group, input, .. }) = self.renaming.take() else {
            return;
        };
        if restore_focus {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
        if !accept {
            return;
        }
        let to = input.read(cx).value().trim().to_owned();
        if to.is_empty() || to == group {
            return;
        }
        // Show the new state at once; the old group's folded state wins a merge.
        let old_collapsed = self.collapsed.contains(&group);
        let target_collapsed = self.collapsed.contains(&to);
        if old_collapsed {
            self.collapsed.insert(to.clone());
        } else {
            self.collapsed.remove(&to);
        }
        let renamed = self.connections.update(cx, |connections, cx| {
            connections.rename_group(group.clone(), to.clone(), cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = renamed.await;
            let failed = result.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    if target_collapsed {
                        this.collapsed.insert(to.clone());
                    } else {
                        this.collapsed.remove(&to);
                    }
                } else {
                    this.collapsed.remove(&group);
                }
                cx.notify();
            });
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    nocterm_ui::notice::error(
                        window,
                        cx,
                        "connections-rename-group",
                        "Could not rename group",
                        error,
                    )
                });
            }
        })
        .detach();
    }

    fn toggle_group(&mut self, group: &str, cx: &mut Context<Self>) {
        if self.renaming.as_ref().is_some_and(|r| r.group == group) {
            return;
        }
        if !self.collapsed.remove(group) {
            self.collapsed.insert(group.to_owned());
        }
        cx.notify();
    }

    fn move_profile(
        &mut self,
        dragged: &DraggedProfile,
        group: Option<String>,
        before: Option<ProfileId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let moved = self.connections.update(cx, |connections, cx| {
            connections.place_profile(dragged.0, group, before, cx)
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Err(error) = moved.await {
                let _ = cx.update(|window, cx| {
                    nocterm_ui::notice::error(
                        window,
                        cx,
                        "connections-move",
                        "Could not move connection",
                        error,
                    )
                });
            }
        })
        .detach();
    }

    fn render_row(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let id = profile.id;
        let element_id = SharedString::from(format!("connection-{id}"));
        let name = profile.name.clone();
        let group = profile.group.clone();
        let facts = ServerFacts::global(cx).read(cx);
        let detected = facts.os(id);
        let flag = facts
            .shown_country(profile, cx)
            .and_then(|code| Some((code.to_uppercase(), facts.flag(code)?)));
        let icon = server_icon(
            profile.icon.as_deref().and_then(os::find).or(detected),
            profile
                .icon_color
                .as_deref()
                .filter(|color| os::is_valid_color(color)),
            theme.muted_foreground,
        );

        h_flex()
            .id(element_id)
            .test_support()
            .group(ROW_GROUP)
            .gap_2()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .on_drag(DraggedProfile(id), move |_, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| nocterm_ui::DragPreview::new(name.clone(), 1, IconName::Server))
            })
            // Dropping on a row files the dragged connection just above it.
            .drag_over::<DraggedProfile>(|style, _, _, cx| {
                style.border_t_2().border_color(cx.theme().primary)
            })
            .on_drop(
                cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, group.clone(), Some(id), window, cx)
                }),
            )
            .when(!profile.description.is_empty(), |row| {
                let description = description_tooltip(&profile.description);
                row.tooltip(move |_, cx| {
                    cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(description.clone()))
                        .into()
                })
            })
            .child(icon)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(profile.name.clone()))
                            .when_some(flag, |line, (country, flag)| {
                                line.child(
                                    div()
                                        .id(SharedString::from(format!("connection-flag-{id}")))
                                        .flex_shrink_0()
                                        .rounded_xs()
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(theme.border)
                                        .child(
                                            img(flag)
                                                .w(gpui_kit::px(16.))
                                                .h(gpui_kit::px(11.))
                                                .object_fit(ObjectFit::Cover),
                                        )
                                        .tooltip(move |_, cx| {
                                            cx.new(|_| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    country.clone(),
                                                )
                                            })
                                            .into()
                                        }),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(profile.target.to_string()),
                    )
                    .when(!profile.description.is_empty(), |column| {
                        column.child(
                            div()
                                .text_xs()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(description_preview(&profile.description)),
                        )
                    }),
            )
            .child(
                h_flex()
                    .invisible()
                    .group_hover(ROW_GROUP, |buttons| buttons.visible())
                    .child(
                        Button::new(SharedString::from(format!("edit-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Pencil)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.edit(id, window, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Trash)
                            .tooltip("Delete")
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.confirm_delete(id, window, cx);
                            })),
                    ),
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.open(id, window, cx)),
            )
    }

    fn render_group_header(
        &self,
        group: &str,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let name = group.to_owned();
        let drop_group = name.clone();
        let renaming = self.renaming.as_ref().filter(|r| r.group == group);
        let label = match renaming {
            Some(rename) => {
                let group = name.clone();
                div()
                    .id(SharedString::from(format!("group-rename-{group}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&rename.input).small())
                    .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                    .on_key_down(
                        cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.finish_rename(false, true, window, cx);
                            }
                        }),
                    )
                    .into_any_element()
            }
            None => {
                let group = name.clone();
                div()
                    .id(SharedString::from(format!("group-name-{name}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(name.clone())
                    // Keep the header from also treating the second click as a toggle.
                    .on_mouse_down(MouseButton::Left, |event, _, cx| {
                        if event.click_count == 2 {
                            cx.stop_propagation();
                        }
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() == 2 {
                            // Undo the first click's toggle, then edit.
                            cx.stop_propagation();
                            this.toggle_group(&group, cx);
                            this.start_rename(&group, window, cx);
                        }
                    }))
                    .into_any_element()
            }
        };
        let toggle_group = name.clone();
        let delete_group = name.clone();
        h_flex()
            .id(SharedString::from(format!("group-{group}")))
            .test_support()
            .group(GROUP_HEADER)
            .gap_1()
            .mx_1()
            .px_1()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .hover(|row| row.bg(theme.sidebar_accent))
            .drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, Some(drop_group.clone()), None, window, cx)
                }),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall(),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::FolderClosed
                } else {
                    IconName::FolderOpen
                })
                .small(),
            )
            .child(label)
            .when(renaming.is_none(), |header| {
                header.child(
                    h_flex()
                        .invisible()
                        .group_hover(GROUP_HEADER, |buttons| buttons.visible())
                        .child(
                            Button::new(SharedString::from(format!("group-toggle-{group}")))
                                .ghost()
                                .xsmall()
                                .icon(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .tooltip(if collapsed { "Expand" } else { "Collapse" })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_group(&toggle_group, cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("group-delete-{group}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip("Delete group")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.confirm_delete_group(delete_group.clone(), window, cx);
                                })),
                        ),
                )
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_group(&name, cx)))
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter_text(cx);
        let filtering = !filter.trim().is_empty();
        let connections = self.connections.read(cx);
        let profiles = connections.profiles();

        let (top, groups) = profiles.grouped(&filter);
        let top: Vec<_> = top.into_iter().cloned().collect();
        let groups: Vec<_> = groups
            .into_iter()
            .map(|(name, members)| (name, members.into_iter().cloned().collect::<Vec<_>>()))
            .collect();
        let empty = profiles.is_empty() && groups.is_empty();
        let nothing_matches = top.is_empty() && groups.is_empty();

        let mut list = v_flex().gap_px().py_1().child(
            h_flex()
                .id("connections-ungrouped")
                .test_support()
                .mx_1()
                .px_2()
                .py_1()
                .rounded(cx.theme().radius)
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Server).small())
                .child("Ungrouped")
                .drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
                .on_drop(cx.listener(|this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, None, None, window, cx)
                })),
        );
        for profile in &top {
            list = list.child(self.render_row(profile, cx));
        }
        for (group, members) in &groups {
            // A filter shows every match, folded or not.
            let collapsed = !filtering && self.collapsed.contains(group);
            list = list.child(self.render_group_header(group, collapsed, cx));
            if !collapsed {
                let drop_group = group.clone();
                let mut folder = v_flex()
                    .id(SharedString::from(format!("group-members-{group}")))
                    .test_support()
                    .pl_3()
                    .gap_px()
                    .drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                            this.move_profile(dragged, Some(drop_group.clone()), None, window, cx)
                        }),
                    );
                for profile in members {
                    folder = folder.child(self.render_row(profile, cx));
                }
                list = list.child(folder);
            }
        }

        if empty {
            self.render_empty(cx).into_any_element()
        } else if nothing_matches {
            div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No connection matches.")
                .into_any_element()
        } else {
            div()
                .id("connections-list")
                .size_full()
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        }
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_3()
            .gap_2()
            .items_start()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("No saved connections yet.")
            .child(
                Button::new("empty-new-connection")
                    .small()
                    .icon(IconName::ServerPlus)
                    .label("New Connection")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        open_editor(None, this.workspace.clone(), window, cx);
                    })),
            )
    }
}

impl Panel for ConnectionsPanel {
    fn title(&self, _: &App) -> SharedString {
        "Connections".into()
    }

    fn icon(&self, _: &App) -> IconName {
        IconName::Server
    }
}

impl Focusable for ConnectionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ConnectionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let load_error = self
            .connections
            .read(cx)
            .load_error()
            .or_else(|| self.connections.read(cx).persistence_error())
            .map(str::to_owned);
        let theme = cx.theme();
        let danger = theme.danger;

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .pb_1()
                    .child(
                        div().flex_1().child(
                            Input::new(&self.filter)
                                .small()
                                .cleanable(true)
                                .prefix(Icon::new(IconName::Search).small()),
                        ),
                    )
                    .child(
                        Button::new("new-connection")
                            .ghost()
                            .small()
                            .icon(IconName::ServerPlus)
                            .tooltip("New Connection")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                open_editor(None, this.workspace.clone(), window, cx);
                            })),
                    ),
            )
            .when_some(load_error, |panel, error| {
                panel.child(
                    div()
                        .mx_2()
                        .p_2()
                        .text_xs()
                        .text_color(danger)
                        .child(format!("Connections: {error}")),
                )
            })
            .child(div().flex_1().min_h_0().child(self.render_list(cx)))
    }
}

#[cfg(test)]
mod tests;
