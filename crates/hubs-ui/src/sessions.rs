//! The sessions panel of a hub window: the machines of the organization and the terminal
//! sessions on them. A session opens as a tab; ending it stops its process.

mod new_session;

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::{HubStatus, HubWindow, HubWindowEvent, Hubs, Load, Watch};
use nocterm_hubs_model::{HubSessionState, HubSessionSummary, NodeSummary, OrgId, OrgSummary};
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use crate::messages::hub_problem;
use crate::plugin::SESSIONS;
use crate::tabs::HubTabs;

pub(crate) use new_session::NewSessionMenu;

pub(crate) struct SessionsPanel {
    hubs: Entity<Hubs>,
    state: Entity<HubWindow>,
    tabs: HubTabs,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    /// Held while the panel shows, so the lists refresh on their own.
    watch: Option<Watch>,
    _subscriptions: Vec<Subscription>,
}

impl SessionsPanel {
    pub(crate) fn new(
        hubs: Entity<Hubs>,
        state: Entity<HubWindow>,
        tabs: HubTabs,
        workspace: &Entity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&hubs, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |_, _, _: &HubWindowEvent, cx| cx.notify()),
            cx.observe(workspace, |panel, _, cx| panel.follow_visibility(cx)),
        ];
        let mut panel = Self {
            hubs,
            state,
            tabs,
            workspace: workspace.downgrade(),
            focus: cx.focus_handle(),
            watch: None,
            _subscriptions: subscriptions,
        };
        panel.follow_visibility(cx);
        panel
    }

    fn shown(&self, cx: &App) -> bool {
        self.workspace
            .read_with(cx, |workspace, _| workspace.panel_is_open(&SESSIONS.id))
            .unwrap_or(false)
    }

    /// Reads the lists every few seconds while the panel shows, and not while it is hidden.
    fn follow_visibility(&mut self, cx: &mut Context<Self>) {
        match (self.shown(cx), self.watch.is_some()) {
            (true, false) => {
                self.watch = Some(self.state.update(cx, |state, cx| {
                    state.refresh(cx);
                    state.watch(cx)
                }));
            }
            (false, true) => self.watch = None,
            _ => {}
        }
    }

    fn status(&self, cx: &App) -> Option<HubStatus> {
        self.hubs.read(cx).status(self.state.read(cx).hub())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.refresh(cx));
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let hub = self.state.read(cx).hub().clone();
        self.hubs.update(cx, |hubs, cx| hubs.retry(&hub, cx));
    }

    fn select_org(&mut self, org: OrgId, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.select_org(org, cx));
    }

    fn open_new_session_menu(&mut self, cx: &mut Context<Self>) {
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.show_new_tab_menu(true, cx));
    }

    /// Asks before ending a session: its process stops and its screen is lost.
    fn confirm_end(
        &mut self,
        session: HubSessionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (state, id) = (state.clone(), session.id.clone());
            let end = Button::new("end-session-confirm")
                .danger()
                .label("End session")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    let ending = state.update(cx, |state, cx| state.end(id.clone(), cx));
                    let message = window.window_handle();
                    cx.spawn(async move |cx| {
                        if let Err(error) = ending.await {
                            let _ = message.update(cx, |_, window, cx| {
                                nocterm_ui::notice::error(
                                    window,
                                    cx,
                                    "hub-end-session",
                                    "Could not end the session",
                                    hub_problem(&error),
                                );
                            });
                        }
                    })
                    .detach();
                });
            let keep = Button::new("end-session-keep")
                .primary()
                .label("Keep it")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                });
            alert
                .title("End this session?")
                .description(format!(
                    "\"{}\" on {} stops, and what it was running is lost.",
                    session.title, session.node_name
                ))
                .on_ok(|_, _, _| true)
                .footer(DialogFooter::new().child(keep).child(end))
        });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let orgs: Vec<OrgSummary> = state
            .account()
            .map(|account| account.orgs.clone())
            .unwrap_or_default();
        let selected = state.org().cloned();
        let muted = cx.theme().muted_foreground;
        let online = self.status(cx).is_some_and(|status| status.is_online());
        v_flex()
            .gap_1()
            .px_2()
            .py_1()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_semibold()
                            .text_color(muted)
                            .child("SESSIONS"),
                    )
                    .child(
                        Button::new("hub-sessions-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("hub-sessions-new")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New session")
                            .disabled(!online)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.open_new_session_menu(cx);
                            })),
                    ),
            )
            .when(orgs.len() > 1, |header| {
                header.child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .children(orgs.into_iter().map(|org| {
                            let id = org.id.clone();
                            Button::new(SharedString::from(format!("hub-org-{}", org.id)))
                                .xsmall()
                                .ghost()
                                .selected(selected.as_ref() == Some(&org.id))
                                .label(org.name)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.select_org(id.clone(), cx);
                                }))
                        })),
                )
            })
    }

    fn render_notice(
        &self,
        text: impl Into<SharedString>,
        retry: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        v_flex()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(text.into()),
            )
            .when(retry, |notice| {
                notice.child(
                    Button::new("hub-sessions-retry")
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.retry(cx);
                            this.refresh(cx);
                        })),
                )
            })
            .into_any_element()
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match self.status(cx) {
            None => {
                return self.render_notice("This server is not in the list any more.", false, cx);
            }
            Some(HubStatus::Offline { .. }) => {
                return self.render_notice(
                    "The server does not answer. Nocterm keeps trying.",
                    true,
                    cx,
                );
            }
            Some(HubStatus::SignedOut | HubStatus::Revoked) => {
                return self.render_notice(
                    "Sign in to this server again from the environment menu.",
                    false,
                    cx,
                );
            }
            Some(HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. }) => {
                return self.render_notice(
                    "This server cannot be used now. See the environment menu.",
                    false,
                    cx,
                );
            }
            Some(HubStatus::Connecting) => return self.render_notice("Connecting…", false, cx),
            Some(HubStatus::Online) => {}
        }
        let state = self.state.read(cx);
        match state.load() {
            Load::Idle | Load::Loading => {
                return self.render_notice("Loading sessions…", false, cx);
            }
            Load::Failed(error) if state.nodes().is_empty() => {
                let text = hub_problem(error);
                return self.render_notice(text, true, cx);
            }
            Load::Failed(_) | Load::Loaded => {}
        }
        if state.nodes().is_empty() {
            return self.render_notice(
                "No machine is connected to this organization yet.",
                false,
                cx,
            );
        }
        let nodes = state.nodes().to_vec();
        let sessions = state.sessions().to_vec();
        v_flex()
            .children(nodes.iter().map(|node| {
                let own: Vec<&HubSessionSummary> = sessions
                    .iter()
                    .filter(|session| session.node == node.id)
                    .collect();
                self.render_node(node, &own, cx)
            }))
            .into_any_element()
    }

    fn render_node(
        &self,
        node: &NodeSummary,
        sessions: &[&HubSessionSummary],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let (muted, ok) = (theme.muted_foreground, theme.success);
        v_flex()
            .id(SharedString::from(format!("hub-node-{}", node.id)))
            .test_support()
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .items_center()
                    .child(
                        Icon::new(if node.online {
                            IconName::Server
                        } else {
                            IconName::ServerOff
                        })
                        .small()
                        .text_color(if node.online { ok } else { muted }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child(node.name.clone()),
                    )
                    .when(!node.online, |row| {
                        row.child(div().text_xs().text_color(muted).child("offline"))
                    }),
            )
            .when(sessions.is_empty(), |node| {
                node.child(
                    div()
                        .px_6()
                        .pb_1()
                        .text_xs()
                        .text_color(muted)
                        .child("No sessions"),
                )
            })
            .children(
                sessions
                    .iter()
                    .map(|session| self.render_session(session, cx)),
            )
            .into_any_element()
    }

    fn render_session(
        &self,
        session: &HubSessionSummary,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.accent);
        let attachable = matches!(
            session.state,
            HubSessionState::Starting | HubSessionState::Running
        ) && session.node_online;
        let mine = self.state.read(cx).is_mine(session);
        let state = match session.state {
            HubSessionState::Starting => "starting",
            HubSessionState::Running => "running",
            HubSessionState::Exited => "exited",
            HubSessionState::Lost => "lost",
        };
        let (open, end, row) = (session.clone(), session.clone(), session.clone());
        let title = if session.title.is_empty() {
            "Session".to_owned()
        } else {
            session.title.clone()
        };
        h_flex()
            .id(SharedString::from(format!("hub-session-{}", session.id)))
            .test_support()
            .gap_2()
            .pl_6()
            .pr_2()
            .py_1()
            .items_center()
            .rounded(theme.radius)
            .when(attachable, |row| {
                row.cursor_pointer().hover(|style| style.bg(hover))
            })
            .child(
                Icon::new(IconName::SquareTerminal)
                    .small()
                    .text_color(muted),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_sm().child(title))
                    .child(div().text_xs().text_color(muted).child(if mine {
                        state.to_owned()
                    } else {
                        format!("{state} · another user")
                    })),
            )
            .when(attachable, |line| {
                line.child(
                    Button::new(SharedString::from(format!("hub-session-open-{}", open.id)))
                        .ghost()
                        .xsmall()
                        .label("Open")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.tabs.open_existing(&open, window, cx);
                        })),
                )
            })
            .when(mine, |line| {
                line.child(
                    Button::new(SharedString::from(format!("hub-session-end-{}", end.id)))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("End session")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.confirm_end(end.clone(), window, cx);
                        })),
                )
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if attachable && event.click_count() == 2 {
                    this.tabs.open_existing(&row, window, cx);
                }
            }))
            .into_any_element()
    }
}

impl Panel for SessionsPanel {
    fn title(&self, _: &App) -> SharedString {
        SESSIONS.title.into()
    }

    fn icon(&self, _: &App) -> IconName {
        IconName::SquareTerminal
    }

    /// How many sessions run on the machines of the organization; none shows no count.
    fn badge(&self, cx: &App) -> Option<SharedString> {
        let running = self
            .state
            .read(cx)
            .sessions()
            .iter()
            .filter(|session| session.state == HubSessionState::Running)
            .count();
        (running > 0).then(|| running.to_string().into())
    }
}

impl Focusable for SessionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SessionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("hub-sessions-panel")
            .track_focus(&self.focus)
            .size_full()
            .child(self.render_header(cx))
            .child(
                div()
                    .id("hub-sessions-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_body(cx)),
            )
    }
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
