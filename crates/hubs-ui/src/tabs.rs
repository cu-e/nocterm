//! Opening terminal tabs on the sessions of a hub.

use std::rc::Rc;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::{self, Either};
use gpui_kit::{App, Entity, WeakEntity, Window};
use nocterm_hubs::{HubWindow, Hubs};
use nocterm_hubs_model::{
    HubId, HubSessionId, HubSessionSummary, HubTerminalTarget, NodeId, NodeSummary,
};
use nocterm_session::{ConnectRequest, Session, Target, Transport};
use nocterm_workspace::Workspace;

use crate::ports::HubTerminalOpener;
use crate::remembered::{Remembered, TabId};

/// Where the tabs of one hub open: its workspace, and what makes the tab.
#[derive(Clone)]
pub(crate) struct TabSite {
    hubs: Entity<Hubs>,
    hub: HubId,
    workspace: WeakEntity<Workspace>,
    opener: Rc<dyn HubTerminalOpener>,
}

impl TabSite {
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
}

/// Opens tabs of one hub in one workspace, and tells the window's memory about them.
#[derive(Clone)]
pub(crate) struct HubTabs {
    site: TabSite,
    remembered: Entity<Remembered>,
}

/// A transport that lets go of its lease when the tab it belongs to is closed.
struct LeasedTransport {
    inner: Arc<dyn Transport>,
    _lease: oneshot::Sender<()>,
}

impl Transport for LeasedTransport {
    fn open(&self, request: ConnectRequest) -> Session {
        self.inner.open(request)
    }
}

impl HubTabs {
    pub(crate) fn new(site: TabSite, remembered: Entity<Remembered>) -> Self {
        Self { site, remembered }
    }

    /// Attaches a tab to a session that runs.
    pub(crate) fn open_existing(
        &self,
        session: &HubSessionSummary,
        window: &mut Window,
        cx: &mut App,
    ) {
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
        );
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
        let site = &self.site;
        site.hubs.update(cx, |hubs, cx| hubs.window(&site.hub, cx))
    }

    fn open(&self, title: String, target: HubTerminalTarget, window: &mut Window, cx: &mut App) {
        let Some(state) = self.state(cx) else {
            return;
        };
        let known = match &target {
            HubTerminalTarget::Existing(session) => Some(session.clone()),
            HubTerminalTarget::New { .. } => None,
        };
        let (transport, created) = state.read(cx).terminal(target).into_parts();
        let (lease, closed) = oneshot::channel();
        let transport: Arc<dyn Transport> = Arc::new(LeasedTransport {
            inner: transport,
            _lease: lease,
        });
        let user = state
            .read(cx)
            .account()
            .map_or_else(|| "hub".to_owned(), |account| account.username.clone());
        let Some(address) = self
            .site
            .hubs
            .read(cx)
            .record(&self.site.hub)
            .map(|r| r.address.clone())
        else {
            return;
        };
        let destination = Target::new(user, address.host(), address.port());
        let opener = Rc::clone(&self.site.opener);
        let opened = self.site.workspace.update(cx, |workspace, cx| {
            opener.open(workspace, title.into(), destination, transport, window, cx)
        });
        match opened {
            Ok(Ok(())) => {
                let tab = self
                    .remembered
                    .update(cx, |remembered, cx| remembered.tab_opened(known, cx));
                self.follow(tab, created, closed, cx);
            }
            Ok(Err(error)) => {
                nocterm_ui::notice::error(
                    window,
                    cx,
                    "hub-open-tab",
                    "Could not open the session",
                    error.to_string(),
                );
            }
            Err(_) => {}
        }
    }

    /// Tells the memory which session a new tab became and when the tab is closed. The transport
    /// is dropped with the tab, which ends `closed`. A window dropped meanwhile has no memory
    /// left to tell, so tabs lost with it do not forget their sessions.
    fn follow(
        &self,
        tab: TabId,
        created: Option<oneshot::Receiver<HubSessionId>>,
        closed: oneshot::Receiver<()>,
        cx: &mut App,
    ) {
        let remembered = self.remembered.downgrade();
        cx.spawn(async move |cx| {
            let closed = match created {
                Some(created) => match future::select(created, closed).await {
                    Either::Left((made, rest)) => {
                        if let Ok(session) = made {
                            let _ = remembered.update(cx, |remembered, cx| {
                                remembered.tab_attached(tab, session, cx);
                            });
                        }
                        Some(rest)
                    }
                    Either::Right(_) => None,
                },
                None => Some(closed),
            };
            if let Some(closed) = closed {
                // Nothing is ever sent: the wait ends when the transport is dropped.
                let _ = closed.await;
            }
            let _ = remembered.update(cx, |remembered, cx| remembered.tab_closed(tab, cx));
        })
        .detach();
    }
}
