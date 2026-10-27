//! Opening terminal tabs on the sessions of a hub.

use std::rc::Rc;

use gpui_kit::{App, Entity, WeakEntity, Window};
use nocterm_hubs::{HubWindow, Hubs};
use nocterm_hubs_model::{HubId, HubSessionSummary, HubTerminalTarget, NodeId, NodeSummary};
use nocterm_session::Target;
use nocterm_workspace::Workspace;

use crate::ports::HubTerminalOpener;

/// Opens tabs of one hub in one workspace.
#[derive(Clone)]
pub(crate) struct HubTabs {
    hubs: Entity<Hubs>,
    hub: HubId,
    workspace: WeakEntity<Workspace>,
    opener: Rc<dyn HubTerminalOpener>,
}

impl HubTabs {
    pub(crate) fn new(
        hubs: Entity<Hubs>,
        hub: HubId,
        workspace: WeakEntity<Workspace>,
        opener: Rc<dyn HubTerminalOpener>,
    ) -> Self {
        Self {
            hubs,
            hub,
            workspace,
            opener,
        }
    }

    /// Attaches a tab to a session that runs; false when the tab could not open.
    pub(crate) fn open_existing(
        &self,
        session: &HubSessionSummary,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let title = if session.title.is_empty() {
            "Session".to_owned()
        } else {
            session.title.clone()
        };
        self.open(
            title,
            HubTerminalTarget::Existing(session.id.clone()),
            window,
            cx,
        )
    }

    /// Starts a shell on `node`, or on a machine the hub picks, and attaches a tab to it.
    pub(crate) fn open_new(&self, node: Option<&NodeSummary>, window: &mut Window, cx: &mut App) {
        let title = node.map_or_else(
            || "Shell".to_owned(),
            |node| format!("Shell on {}", node.name),
        );
        let node: Option<NodeId> = node.map(|node| node.id.clone());
        let Some(state) = self.state(cx) else {
            return;
        };
        let Some(target) = state.read(cx).new_session(node, &title) else {
            nocterm_ui::notice::warning(
                window,
                cx,
                "hub-new-session",
                "Sessions are not loaded yet",
                "Try again in a moment.",
            );
            return;
        };
        self.open(title, target, window, cx);
    }

    fn state(&self, cx: &mut App) -> Option<Entity<HubWindow>> {
        self.hubs.update(cx, |hubs, cx| hubs.window(&self.hub, cx))
    }

    /// Opens the tab of a session; false when it did not open.
    fn open(
        &self,
        title: String,
        target: HubTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(state) = self.state(cx) else {
            return false;
        };
        let (transport, _) = state.read(cx).terminal(target).into_parts();
        let user = state
            .read(cx)
            .account()
            .map_or_else(|| "hub".to_owned(), |account| account.username.clone());
        let Some(address) = self
            .hubs
            .read(cx)
            .record(&self.hub)
            .map(|r| r.address.clone())
        else {
            return false;
        };
        let destination = Target::new(user, address.host(), address.port());
        let opener = Rc::clone(&self.opener);
        let opened = self.workspace.update(cx, |workspace, cx| {
            opener.open(workspace, title.into(), destination, transport, window, cx)
        });
        match opened {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                nocterm_ui::notice::error(
                    window,
                    cx,
                    "hub-open-tab",
                    "Could not open the session",
                    error.to_string(),
                );
                false
            }
            Err(_) => false,
        }
    }
}
