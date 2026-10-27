//! The sessions of one hub: the machines of the organization and the terminal sessions on
//! them. A session opens as a tab; ending it stops its process.

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString,
    Subscription, TestSupportExt as _, WeakEntity, Window,
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
use nocterm_hubs_model::{
    HubSessionId, HubSessionState, HubSessionSummary, NodeSummary, OrgId, OrgSummary,
};
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;

use crate::messages::hub_problem;
use crate::plugin::SESSIONS;
use crate::tabs::HubTabs;
use crate::{problems, sign_in};

pub(super) struct SessionList {
    hubs: Entity<Hubs>,
    state: Entity<HubWindow>,
    tabs: HubTabs,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    /// The session Enter opens.
    selected: Option<HubSessionId>,
    /// Held while the panel shows, so the lists refresh on their own.
    watch: Option<Watch>,
    _subscriptions: Vec<Subscription>,
}

impl SessionList {
    pub(super) fn new(
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
        // The workspace creates the panel while it is being updated, so it is read on the next turn.
        cx.spawn(async move |panel, cx| {
            let _ = panel.update(cx, |panel, cx| panel.follow_visibility(cx));
        })
        .detach();
        Self {
            hubs,
            state,
            tabs,
            workspace: workspace.downgrade(),
            focus: cx.focus_handle(),
            selected: None,
            watch: None,
            _subscriptions: subscriptions,
        }
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

    /// The sessions a tab can attach to, in the order the panel lists them.
    fn attachable(&self, cx: &App) -> Vec<HubSessionSummary> {
        let state = self.state.read(cx);
        state
            .nodes()
            .iter()
            .flat_map(|node| {
                state
                    .sessions()
                    .iter()
                    .filter(|session| session.node == node.id && is_attachable(session))
            })
            .cloned()
            .collect()
    }

    fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) {
        let sessions = self.attachable(cx);
        let current = sessions
            .iter()
            .position(|session| Some(&session.id) == self.selected.as_ref());
        let next = match (current, down) {
            (None, true) => 0,
            (None, false) => sessions.len().wrapping_sub(1),
            (Some(index), true) => (index + 1) % sessions.len(),
            (Some(index), false) => (index + sessions.len() - 1) % sessions.len(),
        };
        self.selected = sessions.get(next).map(|session| session.id.clone());
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            key @ ("up" | "down") if !self.attachable(cx).is_empty() => {
                self.move_selection(key == "down", cx);
                cx.stop_propagation();
            }
            "enter" if self.selected.is_some() => {
                self.open_selected(window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .attachable(cx)
            .into_iter()
            .find(|session| Some(&session.id) == self.selected.as_ref());
        if let Some(session) = selected {
            self.tabs.open_existing(&session, window, cx);
        }
    }

    /// Starts a shell on `node`, or on a machine the hub picks.
    fn open_new(
        &mut self,
        node: Option<&NodeSummary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.open_new(node, window, cx);
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hub = self.state.read(cx).hub().clone();
        sign_in::start(&self.hubs, Some(&hub), window, cx);
    }

    /// Explains why the server cannot be used: its certificate or its protocol.
    fn explain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hub = self.state.read(cx).hub().clone();
        let Some(summary) = self.hubs.read(cx).summary(&hub) else {
            return;
        };
        match summary.status.clone() {
            HubStatus::TrustFailed { pinned, presented } => {
                problems::open_trust_failed(
                    self.hubs.clone(),
                    summary,
                    pinned,
                    presented,
                    window,
                    cx,
                );
            }
            HubStatus::UpgradeRequired { hub, client } => {
                problems::open_upgrade_required(&summary, hub, client, window, cx);
            }
            _ => {}
        }
    }

    /// How many sessions run on the machines of the organization.
    pub(super) fn running(&self, cx: &App) -> usize {
        self.state
            .read(cx)
            .sessions()
            .iter()
            .filter(|session| session.state == HubSessionState::Running)
            .count()
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
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_new(None, window, cx);
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
        offer: Offer,
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
            .when(offer == Offer::Retry, |notice| {
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
            .when(offer == Offer::Explain, |notice| {
                notice.child(
                    Button::new("hub-sessions-explain")
                        .small()
                        .label("Why?")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.explain(window, cx);
                        })),
                )
            })
            .when(offer == Offer::SignIn, |notice| {
                notice.child(
                    Button::new("hub-sessions-sign-in")
                        .small()
                        .label("Sign in")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.sign_in(window, cx);
                        })),
                )
            })
            .into_any_element()
    }

    fn render_body(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match self.status(cx) {
            None => {
                return self.render_notice(
                    "This server is not in the list any more.",
                    Offer::Nothing,
                    cx,
                );
            }
            Some(HubStatus::Offline { .. }) => {
                return self.render_notice(
                    "The server does not answer. Nocterm keeps trying.",
                    Offer::Retry,
                    cx,
                );
            }
            Some(HubStatus::SignedOut | HubStatus::Revoked) => {
                return self.render_notice(
                    "This device is not signed in to this server.",
                    Offer::SignIn,
                    cx,
                );
            }
            Some(HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. }) => {
                return self.render_notice("This server cannot be used now.", Offer::Explain, cx);
            }
            Some(HubStatus::Connecting) => {
                return self.render_notice("Connecting…", Offer::Nothing, cx);
            }
            Some(HubStatus::Online) => {}
        }
        let state = self.state.read(cx);
        match state.load() {
            Load::Idle | Load::Loading => {
                return self.render_notice("Loading sessions…", Offer::Nothing, cx);
            }
            Load::Failed(error) if state.nodes().is_empty() => {
                let text = hub_problem(error);
                return self.render_notice(text, Offer::Retry, cx);
            }
            Load::Failed(_) | Load::Loaded => {}
        }
        if state.nodes().is_empty() {
            return self.render_notice(
                "No machine is connected to this organization yet.",
                Offer::Nothing,
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
                    })
                    .when(node.online, |row| row.child(self.new_button(node, cx))),
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
        let attachable = is_attachable(session);
        let selected = self.selected.as_ref() == Some(&session.id);
        let mine = self.state.read(cx).is_mine(session);
        let row = session.clone();
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
            .when(selected, |row| row.bg(hover))
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
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(state_text(session, mine)),
                    ),
            )
            .when(attachable, |line| line.child(self.open_button(session, cx)))
            .when(mine, |line| line.child(self.end_button(session, cx)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if !attachable {
                    return;
                }
                // Enter opens the session that was clicked last.
                this.selected = Some(row.id.clone());
                window.focus(&this.focus, cx);
                if event.click_count() == 2 {
                    this.tabs.open_existing(&row, window, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn new_button(&self, node: &NodeSummary, cx: &mut Context<Self>) -> Button {
        let node = node.clone();
        Button::new(SharedString::from(format!("hub-node-new-{}", node.id)))
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .tooltip(format!("New session on {}", node.name))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_new(Some(&node), window, cx);
            }))
    }

    fn open_button(&self, session: &HubSessionSummary, cx: &mut Context<Self>) -> Button {
        let open = session.clone();
        Button::new(SharedString::from(format!("hub-session-open-{}", open.id)))
            .ghost()
            .xsmall()
            .label("Open")
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.tabs.open_existing(&open, window, cx);
            }))
    }

    fn end_button(&self, session: &HubSessionSummary, cx: &mut Context<Self>) -> Button {
        let end = session.clone();
        Button::new(SharedString::from(format!("hub-session-end-{}", end.id)))
            .ghost()
            .xsmall()
            .icon(IconName::X)
            .tooltip("End session")
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.confirm_end(end.clone(), window, cx);
            }))
    }
}

/// What a session row says of the session: its state, and whose it is when not the account's.
fn state_text(session: &HubSessionSummary, mine: bool) -> String {
    let state = match session.state {
        HubSessionState::Starting => "starting",
        HubSessionState::Running => "running",
        HubSessionState::Exited => "exited",
        HubSessionState::Lost => "lost",
    };
    if mine {
        state.to_owned()
    } else {
        format!("{state} · another user")
    }
}

/// What a notice offers to do about what it says.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Offer {
    Nothing,
    Retry,
    SignIn,
    Explain,
}

/// A session a tab can attach to: it runs, on a machine that is online.
fn is_attachable(session: &HubSessionSummary) -> bool {
    matches!(
        session.state,
        HubSessionState::Starting | HubSessionState::Running
    ) && session.node_online
}

impl Focusable for SessionList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SessionList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("hub-sessions-panel")
            .track_focus(&self.focus)
            .key_context("HubSessions")
            .size_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
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
