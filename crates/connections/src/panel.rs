//! The sidebar list of saved connections.

use std::collections::HashSet;

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
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
                    if let Err(error) = deleted {
                        window.push_notification(error, cx);
                    }
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

    fn render_row(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let id = profile.id;
        let element_id = SharedString::from(format!("connection-{id}"));

        h_flex()
            .id(element_id)
            .group(ROW_GROUP)
            .gap_2()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .when(!profile.description.is_empty(), |row| {
                let description = profile.description.clone();
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
                                .child(profile.description.clone()),
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
        h_flex()
            .id(SharedString::from(format!("group-{group}")))
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

        let top: Vec<Profile> = profiles
            .in_group(None, &filter)
            .into_iter()
            .cloned()
            .collect();
        let groups: Vec<(String, Vec<Profile>)> = profiles
            .groups()
            .into_iter()
            .map(|group| {
                let members = profiles
                    .in_group(Some(group), &filter)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>();
                (group.to_owned(), members)
            })
            .filter(|(_, members)| !members.is_empty())
            .collect();
        let empty = profiles.is_empty();
        let nothing_matches = top.is_empty() && groups.is_empty();

        let mut list = v_flex().gap_px().py_1();
        for profile in &top {
            list = list.child(self.render_row(profile, cx));
        }
        for (group, members) in &groups {
            // A filter shows every match, folded or not.
            let collapsed = !filtering && self.collapsed.contains(group);
            list = list.child(self.render_group_header(group, collapsed, cx));
            if !collapsed {
                let mut folder = v_flex().pl_3().gap_px();
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
        let load_error = self.connections.read(cx).load_error().map(str::to_owned);
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
                        .child(format!("Saved connections could not be read. {error}")),
                )
            })
            .child(div().flex_1().min_h_0().child(self.render_list(cx)))
    }
}
