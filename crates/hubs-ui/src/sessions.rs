//! The sessions panel of the window: the sessions of one hub at a time, and the choice of the
//! hub when this device knows several. The sessions of the chosen hub are listed by
//! [`list::SessionList`].

mod list;

use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hubs::{HubSummary, Hubs};
use nocterm_hubs_model::HubId;
use nocterm_ui::IconName;
use nocterm_workspace::{Panel, Workspace};

use crate::plugin::SESSIONS;
use crate::ports::HubTerminalOpener;
use crate::sign_in;
use crate::tabs::HubTabs;
use list::SessionList;

/// The hub the panel lists and the view that lists it.
struct Shown {
    hub: HubId,
    list: Entity<SessionList>,
}

pub(crate) struct SessionsPanel {
    hubs: Entity<Hubs>,
    workspace: WeakEntity<Workspace>,
    opener: Rc<dyn HubTerminalOpener>,
    shown: Option<Shown>,
    /// The hub the user picked; the panel picks one itself until then.
    picked: Option<HubId>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SessionsPanel {
    pub(crate) fn new(
        hubs: Entity<Hubs>,
        opener: Rc<dyn HubTerminalOpener>,
        workspace: &Entity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![cx.observe(&hubs, |panel, _, cx| panel.follow_hubs(cx))];
        let mut panel = Self {
            hubs,
            workspace: workspace.downgrade(),
            opener,
            shown: None,
            picked: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        panel.follow_hubs(cx);
        panel
    }

    /// The hub the panel lists: the one the user picked, else the one it lists already, while
    /// they are known; else the first that answers, else the first.
    fn choice(&self, servers: &[HubSummary]) -> Option<HubId> {
        let known = |hub: &&HubId| servers.iter().any(|server| &server.id == *hub);
        let current = self.shown.as_ref().map(|shown| &shown.hub);
        self.picked
            .as_ref()
            .filter(known)
            .or_else(|| current.filter(known))
            .cloned()
            .or_else(|| {
                servers
                    .iter()
                    .find(|server| server.status.is_online())
                    .or_else(|| servers.first())
                    .map(|server| server.id.clone())
            })
    }

    /// Lists the hub the panel should list now, after the known hubs changed.
    fn follow_hubs(&mut self, cx: &mut Context<Self>) {
        let servers = self.hubs.read(cx).list();
        let choice = self.choice(&servers);
        if self.shown.as_ref().map(|shown| &shown.hub) != choice.as_ref() {
            self.shown = choice.and_then(|hub| self.list_of(hub, cx));
        }
        cx.notify();
    }

    fn list_of(&self, hub: HubId, cx: &mut Context<Self>) -> Option<Shown> {
        let workspace = self.workspace.upgrade()?;
        let state = self
            .hubs
            .update(cx, |hubs, cx| hubs.session_lists(&hub, cx))?;
        let tabs = HubTabs::new(
            self.hubs.clone(),
            hub.clone(),
            self.workspace.clone(),
            Rc::clone(&self.opener),
        );
        let hubs = self.hubs.clone();
        let list = cx.new(|cx| SessionList::new(hubs, state, tabs, &workspace, cx));
        Some(Shown { hub, list })
    }

    fn pick(&mut self, hub: HubId, cx: &mut Context<Self>) {
        self.picked = Some(hub);
        self.follow_hubs(cx);
    }

    fn render_choice(&self, servers: Vec<HubSummary>, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown.as_ref().map(|shown| shown.hub.clone());
        h_flex()
            .gap_1()
            .px_2()
            .pb_1()
            .flex_wrap()
            .children(servers.into_iter().map(|server| {
                let id = server.id.clone();
                Button::new(SharedString::from(format!("hub-choice-{}", server.id)))
                    .xsmall()
                    .ghost()
                    .selected(shown.as_ref() == Some(&server.id))
                    .label(server.label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.pick(id.clone(), cx);
                    }))
            }))
    }

    fn render_no_server(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No server yet. Sign in to a hub to see its sessions."),
            )
            .child(
                Button::new("hub-sessions-add-server")
                    .small()
                    .label("Sign in to a server")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        sign_in::start(&this.hubs, None, window, cx);
                    })),
            )
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

    /// How many sessions run on the machines of the organization shown; none shows no count.
    fn badge(&self, cx: &App) -> Option<SharedString> {
        let running = self.shown.as_ref()?.list.read(cx).running(cx);
        (running > 0).then(|| running.to_string().into())
    }
}

impl Focusable for SessionsPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.shown {
            Some(shown) => shown.list.focus_handle(cx),
            None => self.focus.clone(),
        }
    }
}

impl Render for SessionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let servers = self.hubs.read(cx).list();
        let body = match &self.shown {
            Some(shown) => shown.list.clone().into_any_element(),
            None => self.render_no_server(cx),
        };
        v_flex()
            .id("hub-sessions-root")
            .track_focus(&self.focus)
            .size_full()
            .when(servers.len() > 1, |root| {
                root.child(self.render_choice(servers, cx))
            })
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
