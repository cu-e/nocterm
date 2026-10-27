//! The menu behind "+" in a hub window: start a shell on any machine or on a chosen one.

use gpui_kit::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    WeakEntity, Window,
    base::actions::Confirm,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_hubs::{HubWindow, HubWindowEvent};
use nocterm_hubs_model::NodeSummary;
use nocterm_ui::IconName;
use nocterm_workspace::Workspace;

use crate::tabs::HubTabs;

pub(crate) struct NewSessionMenu {
    state: Entity<HubWindow>,
    tabs: HubTabs,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl NewSessionMenu {
    pub(crate) fn new(
        state: Entity<HubWindow>,
        tabs: HubTabs,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _subscription: cx.subscribe(&state, |_, _, _: &HubWindowEvent, cx| cx.notify()),
            state,
            tabs,
            workspace,
            focus: cx.focus_handle(),
        }
    }

    /// Starts a shell on `node`, or on a machine the hub picks, and closes the menu.
    fn start(&mut self, node: Option<NodeSummary>, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self
            .workspace
            .update(cx, |workspace, cx| workspace.show_new_tab_menu(false, cx));
        self.tabs.open_new(node.as_ref(), window, cx);
    }
}

impl Focusable for NewSessionMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NewSessionMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nodes = self.state.read(cx).nodes().to_vec();
        let any_online = nodes.iter().any(|node| node.online);
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("new-session-menu")
            .track_focus(&self.focus)
            .w(px(260.))
            .gap_1()
            // Enter takes the first choice: any machine.
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.start(None, window, cx)))
            .child(
                div()
                    .px_2()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child("NEW SESSION"),
            )
            .child(
                Button::new("new-session-auto")
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .icon(IconName::SquareTerminal)
                    .label("On any available machine")
                    .disabled(!any_online)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.start(None, window, cx);
                    })),
            )
            .children(nodes.into_iter().enumerate().map(|(index, node)| {
                let online = node.online;
                let label = if online {
                    node.name.clone()
                } else {
                    format!("{} (offline)", node.name)
                };
                Button::new(("new-session-node", index))
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .icon(if online {
                        IconName::Server
                    } else {
                        IconName::ServerOff
                    })
                    .label(SharedString::from(label))
                    .disabled(!online)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.start(Some(node.clone()), window, cx);
                    }))
            }))
    }
}
