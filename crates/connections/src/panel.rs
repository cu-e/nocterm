//! The sidebar list of saved connections.

use nocterm_connections_model::profiles::{Profile, ProfileId};
use std::{collections::HashSet, sync::Arc};

use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, Hsla, Image, ImageFormat,
    MouseButton, SharedString, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
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
    Connections, Installed, open_editor,
    os::{self, Os},
    session_intents::open_profile,
};
use account_state::AccountState;

pub(crate) mod account_state;
mod folder;
mod group_actions;
mod row;
mod sections;
use group_actions::{GroupAction, run_group_action};
use sections::ConnectionList;

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
    installed: Installed,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    filter: Entity<InputState>,
    /// Folders the user folded away.
    collapsed: HashSet<String>,
    renaming: Option<GroupRename>,
    listing: ConnectionList,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionsPanel {
    pub fn new(
        installed: Installed,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let mut subscriptions = vec![
            cx.observe_in(
                installed.connections(),
                window,
                |this, connections, window, cx| {
                    this.listing.invalidate();
                    // The group may have been deleted or ungrouped while being renamed.
                    if let Some(rename) = &this.renaming
                        && !connections.read(cx).profiles(cx).has_group(&rename.group)
                    {
                        let focused = rename.input.read(cx).focus_handle(cx).is_focused(window);
                        this.renaming = None;
                        if focused {
                            window.focus(&this.focus_handle, cx);
                        }
                    }
                    cx.notify();
                },
            ),
            // Detected systems and countries, and flags as they load.
            cx.observe(installed.facts(), |_, _, cx| cx.notify()),
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.listing.invalidate();
                    cx.notify();
                }
                // Enter opens the only match.
                InputEvent::PressEnter { .. } => this.open_single_match(window, cx),
                _ => {}
            }),
        ];
        // Whether the list may be changed follows the account, not only the profiles.
        if let Some(replication) = installed.replication() {
            subscriptions.push(cx.observe(replication, |this, _, cx| {
                this.listing.invalidate();
                cx.notify();
            }));
        }
        // The rows show what the hub's agents found on each server.
        if let Some(access) = installed.inputs().agent_access() {
            let panel = cx.weak_entity();
            let changed = move |cx: &mut App| {
                let _ = panel.update(cx, |_, cx| cx.notify());
            };
            subscriptions.push(access.observe(Box::new(changed), cx));
        }
        Self {
            installed,
            workspace,
            focus_handle: cx.focus_handle(),
            filter,
            collapsed: HashSet::new(),
            renaming: None,
            listing: ConnectionList::default(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the list may be changed now: see [`account_state`].
    fn editable(&self, cx: &App) -> bool {
        AccountState::of(&self.installed, cx).editable()
    }

    fn filter_text(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    fn open(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .get(id)
            .cloned()
        else {
            return;
        };
        open_profile(&self.installed, &self.workspace, &profile, window, cx);
    }

    /// Opens every connection of `group`, each in its tab.
    fn open_group(&mut self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        let members: Vec<_> = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .filter(|profile| profile.group.as_deref() == Some(group))
            .cloned()
            .collect();
        for profile in &members {
            open_profile(&self.installed, &self.workspace, profile, window, cx);
        }
    }

    fn open_single_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter_text(cx);
        let matches: Vec<ProfileId> = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .filter(|profile| profile.matches(&filter))
            .map(|profile| profile.id)
            .collect();
        if let [id] = matches[..] {
            self.open(id, window, cx);
        }
    }

    fn edit(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let profile = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .get(id)
            .cloned();
        if profile.is_some() {
            open_editor(
                self.installed.clone(),
                profile,
                self.workspace.clone(),
                window,
                cx,
            );
        }
    }

    fn confirm_delete(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let Some(name) = self
            .installed
            .connections()
            .read(cx)
            .profiles(cx)
            .get(id)
            .map(|profile| profile.name.clone())
        else {
            return;
        };
        let connections = self.installed.connections().clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let connections = connections.clone();
            alert
                .title(format!("Delete “{name}”?"))
                .description("The saved connection is removed. Open sessions stay open.")
                .ok_text("Delete")
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let deleted = connections
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
                                        error.to_string(),
                                    )
                                });
                            }
                        })
                        .detach();
                    true
                })
        });
    }

    /// The connections in folder `group`.
    fn folder_members(&self, group: &str, cx: &App) -> Vec<ProfileId> {
        let profiles = self.installed.connections().read(cx).profiles(cx);
        profiles
            .in_group(Some(group), "")
            .iter()
            .map(|profile| profile.id)
            .collect()
    }

    fn confirm_delete_group(&mut self, group: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        // The delete only applies while the group still holds these connections.
        let members = self.folder_members(&group, cx);
        let count = members.len();
        let panel = cx.weak_entity();
        let connections = self.installed.connections().clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let description = match count {
                0 => "This group is empty.".to_owned(),
                1 => "1 connection in this group. Delete it, or ungroup to move it to Ungrouped. Open sessions stay open. Saved vault credentials are kept.".to_owned(),
                n => format!("{n} connections in this group. Delete them, or ungroup to move them to Ungrouped. Open sessions stay open. Saved vault credentials are kept."),
            };
            let action = |id: &'static str, label: &'static str, action: GroupAction| {
                let (group, panel, members, connections) = (group.clone(), panel.clone(), members.clone(), connections.clone());
                Button::new(id)
                    .label(label)
                    .when(matches!(action, GroupAction::Delete), |button| button.danger())
                    .on_click(move |_: &ClickEvent, window, cx| {
                        run_group_action(
                            &connections,
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
        if !self.editable(cx) {
            return;
        }
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
        self.listing.invalidate();
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
        self.listing.invalidate();
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
        let members = self.folder_members(&group, cx);
        let renamed = self.installed.connections().update(cx, |connections, cx| {
            connections.rename_group_of(members, group.clone(), to.clone(), cx)
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
                        error.to_string(),
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
        self.listing.invalidate();
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
        if !self.editable(cx) {
            return;
        }
        let moved = self.installed.connections().update(cx, |connections, cx| {
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
                        error.to_string(),
                    )
                });
            }
        })
        .detach();
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter_text(cx);
        let editable = self.editable(cx);
        let profiles = self.installed.connections().read(cx).profiles(cx);
        let rename = self.renaming.as_ref().map(|rename| {
            (
                rename.group.as_str(),
                rename.input.read(cx).focus_handle(cx),
            )
        });
        self.listing
            .prepare(profiles, editable, &filter, &self.collapsed, rename);
        if self.listing.empty() {
            return self.render_empty(cx).into_any_element();
        }
        if self.listing.nothing_matches() {
            return div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No connection matches.")
                .into_any_element();
        }
        let rows = self.listing.rows();
        div()
            .id("connections-list")
            .test_support()
            .size_full()
            .py_1()
            .child(
                gpui_kit::list(
                    self.listing.scroll(),
                    cx.processor(move |this, index: usize, _, cx| rows[index].render(this, cx)),
                )
                .size_full(),
            )
            .into_any_element()
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
                    .disabled(!self.editable(cx))
                    .icon(IconName::ServerPlus)
                    .label("New Connection")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        open_editor(
                            this.installed.clone(),
                            None,
                            this.workspace.clone(),
                            window,
                            cx,
                        );
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
            .installed
            .connections()
            .read(cx)
            .load_error()
            .map(ToString::to_string)
            .or_else(|| {
                self.installed
                    .connections()
                    .read(cx)
                    .persistence_error(cx)
                    .map(|error| error.to_string())
            });
        let account = AccountState::of(&self.installed, cx);
        let editable = account.editable();
        let notice = account.notice();
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
                        div()
                            .id("connections-filter")
                            .test_support()
                            .flex_1()
                            .child(
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
                            .disabled(!editable)
                            .icon(IconName::ServerPlus)
                            .tooltip("New Connection")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                open_editor(
                                    this.installed.clone(),
                                    None,
                                    this.workspace.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when_some(notice, |panel, notice| {
                panel.child(account_state::strip(&notice, cx))
            })
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
