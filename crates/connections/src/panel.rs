//! The sidebar list of saved connections.

use std::collections::HashSet;

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use crate::{
    Connections,
    model::{connect, spec_for_profile},
    open_editor,
    store::{Profile, ProfileId},
};

/// Shared by every row, so hovering a row reveals only its own buttons.
const ROW_GROUP: &str = "connection-row";

#[derive(Clone)]
struct DraggedProfile(ProfileId);

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
            cx.observe(&connections, |_, _, cx| cx.notify()),
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

    fn toggle_group(&mut self, group: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(group) {
            self.collapsed.insert(group.to_owned());
        }
        cx.notify();
    }

    fn move_profile(
        &mut self,
        dragged: &DraggedProfile,
        group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let moved = self.connections.update(cx, |connections, cx| {
            connections.move_profile(dragged.0, group, cx)
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
            .when(!profile.description.is_empty(), |row| {
                let description = description_tooltip(&profile.description);
                row.tooltip(move |_, cx| {
                    cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(description.clone()))
                        .into()
                })
            })
            .child(
                Icon::new(IconName::Server)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(profile.name.clone()))
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
        h_flex()
            .id(SharedString::from(format!("group-{group}")))
            .test_support()
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
                    this.move_profile(dragged, Some(drop_group.clone()), window, cx)
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
            .child(div().truncate().child(name.clone()))
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
                    this.move_profile(dragged, None, window, cx)
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
                            this.move_profile(dragged, Some(drop_group.clone()), window, cx)
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
mod tests {
    use super::*;
    use futures::FutureExt as _;
    use gpui_kit::{TestAppContext, test::TestWindowExt as _};

    #[test]
    fn description_preview_and_tooltip_are_bounded_without_losing_unicode() {
        let long = format!("\n  {}\nsecond line", "я".repeat(1000));
        let preview = description_preview(&long);
        assert_eq!(preview.chars().count(), 160);
        assert!(!preview.contains('\n'));
        let tooltip = description_tooltip(&long);
        assert_eq!(tooltip.chars().count(), 513);
        assert!(tooltip.ends_with('…'));
        assert_eq!(description_preview("\n\t"), "");
    }

    #[gpui_kit::test]
    fn native_drag_moves_to_collapsed_folder_root_and_remembered_empty_folder(
        cx: &mut TestAppContext,
    ) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let profile = |name: &str, group: &str| Profile {
            id: ProfileId::generate(),
            name: name.into(),
            group: Some(group.into()),
            target: nocterm_session::Target::new("ci", name, 22),
            description: String::new(),
            options: Default::default(),
            auth: Default::default(),
            credential: None,
            launch: None,
        };
        let a = profile("a", "Work");
        let b = profile("b", "Personal");
        cx.update_window(handle, |_, window, cx| {
            let connections = Connections::global(cx);
            connections.update(cx, |connections, cx| {
                connections
                    .save_profile(a.clone(), cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
                connections
                    .save_profile(b, cx)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
            });
            let panel = cx.new(|cx| {
                ConnectionsPanel::new(connections.clone(), workspace.downgrade(), window, cx)
            });
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel, cx));
            window.render_frame(cx);
            window.click("group-Personal", cx);
            window.render_frame(cx);
            use gpui_kit::{
                InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
            };
            let from = window
                .find(SharedString::from(format!("connection-{}", a.id)))
                .bounds()
                .center();
            let to = window.find("group-Personal").bounds().center();
            window.dispatch_event(
                MouseMoveEvent {
                    position: from,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseMoveEvent {
                    position: to,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(
                cx.has_active_drag(),
                "a real row creates its typed drag payload"
            );
            let preview = window.find("drag-preview").bounds();
            assert!(
                preview.size.height > gpui_kit::px(20.) && preview.size.height < gpui_kit::px(80.),
                "preview geometry must be independent of terminal font size"
            );
            assert!(
                preview.size.width > gpui_kit::px(40.) && preview.size.width <= gpui_kit::px(320.)
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: to,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(!cx.has_active_drag());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                Connections::global(cx)
                    .read(cx)
                    .profiles()
                    .get(a.id)
                    .unwrap()
                    .group
                    .as_deref(),
                Some("Personal")
            );
            window.render_frame(cx);
            assert!(
                window.try_find("group-Work").is_some(),
                "last-member folder remains a drop target"
            );
            window.click("group-Personal", cx);
            window.render_frame(cx);
            window.drag_to(
                SharedString::from(format!("connection-{}", a.id)),
                "connections-ungrouped",
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                Connections::global(cx)
                    .read(cx)
                    .profiles()
                    .get(a.id)
                    .unwrap()
                    .group
                    .is_none()
            );
            window.render_frame(cx);
            window.drag_to(
                SharedString::from(format!("connection-{}", a.id)),
                "group-Work",
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.read(|cx| {
            assert_eq!(
                Connections::global(cx)
                    .read(cx)
                    .profiles()
                    .get(a.id)
                    .unwrap()
                    .group
                    .as_deref(),
                Some("Work")
            )
        });
        assert!(
            opened.borrow().is_empty(),
            "a drag must not open an SSH session"
        );
    }
}
