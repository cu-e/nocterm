//! What a hub window keeps of its server across a restart: the organization it shows and the
//! sessions its tabs are attached to.
//!
//! The owner is told when a tab opens, learns the id of a session it started, closes, or when
//! the organization changes, and hands the whole memory to the [`HubMemoryKeeper`] when it
//! differs from the last one it gave. After a restart it holds the remembered sessions until
//! the hub lists its sessions, then names the ones that can be attached again.

use std::rc::Rc;

use gpui_kit::{App, Context, Subscription};
use nocterm_hubs_model::{
    HubId, HubSessionId, HubSessionState, HubSessionSummary, HubWindowMemory, OrgId,
};

use crate::ports::HubMemoryKeeper;

/// One open tab, as far as the memory is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TabId(u64);

struct Tab {
    id: TabId,
    /// The session the tab is attached to; not known for a new session until the hub made it.
    session: Option<HubSessionId>,
}

/// The memory of one hub window.
pub(crate) struct Remembered {
    hub: HubId,
    keeper: Rc<dyn HubMemoryKeeper>,
    organization: Option<OrgId>,
    /// Sessions remembered from the previous run that no tab is attached to yet.
    pending: Vec<HubSessionId>,
    tabs: Vec<Tab>,
    next_tab: u64,
    /// What the keeper has: a change is reported only when it differs.
    kept: HubWindowMemory,
    /// The watch on the hub's lists that brings the pending sessions back.
    _watch: Option<Subscription>,
}

impl Remembered {
    /// A window that starts with what the previous run kept of `hub`, which the keeper has
    /// already.
    pub(crate) fn new(
        hub: HubId,
        keeper: Rc<dyn HubMemoryKeeper>,
        restored: HubWindowMemory,
    ) -> Self {
        Self {
            hub,
            keeper,
            organization: restored.organization().cloned(),
            pending: restored.sessions().to_vec(),
            tabs: Vec::new(),
            next_tab: 0,
            kept: restored,
            _watch: None,
        }
    }

    pub(crate) fn watching(&mut self, watch: Subscription) {
        self._watch = Some(watch);
    }

    /// A tab opened on `session`, if that is known already. A session that was waiting to be
    /// attached again is attached now.
    pub(crate) fn tab_opened(
        &mut self,
        session: Option<HubSessionId>,
        cx: &mut Context<Self>,
    ) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        if let Some(session) = &session {
            self.pending.retain(|waiting| waiting != session);
        }
        self.tabs.push(Tab { id, session });
        self.save(cx);
        id
    }

    /// The hub made the session a tab started.
    pub(crate) fn tab_attached(
        &mut self,
        tab: TabId,
        session: HubSessionId,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.tabs.iter_mut().find(|open| open.id == tab) {
            open.session = Some(session);
            self.save(cx);
        }
    }

    pub(crate) fn tab_closed(&mut self, tab: TabId, cx: &mut Context<Self>) {
        self.tabs.retain(|open| open.id != tab);
        self.save(cx);
    }

    /// The window shows `organization` now.
    pub(crate) fn organization_shown(&mut self, organization: OrgId, cx: &mut Context<Self>) {
        if self.organization.as_ref() != Some(&organization) {
            self.organization = Some(organization);
            self.save(cx);
        }
    }

    /// The remembered sessions that `listed`, the sessions the hub runs, still has and that no
    /// tab shows; the others ended meanwhile and are forgotten. Called once the hub has listed
    /// its sessions: what it returns is attached again, and a second call finds nothing.
    pub(crate) fn reattachable(
        &mut self,
        listed: &[HubSessionSummary],
        cx: &mut Context<Self>,
    ) -> Vec<HubSessionSummary> {
        let pending = std::mem::take(&mut self.pending);
        let open = |session: &HubSessionId| {
            self.tabs
                .iter()
                .any(|tab| tab.session.as_ref() == Some(session))
        };
        let found = pending
            .iter()
            .filter(|session| !open(session))
            .filter_map(|session| {
                listed.iter().find(|candidate| {
                    candidate.id == *session
                        && matches!(
                            candidate.state,
                            HubSessionState::Starting | HubSessionState::Running
                        )
                })
            })
            .cloned()
            .collect();
        self.save(cx);
        found
    }

    /// Whether remembered sessions still wait for the hub's list.
    pub(crate) fn is_waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The sessions of the tabs in the order they opened, after those still waiting.
    fn memory(&self) -> HubWindowMemory {
        let mut sessions = self.pending.clone();
        for session in self.tabs.iter().filter_map(|tab| tab.session.as_ref()) {
            if !sessions.contains(session) {
                sessions.push(session.clone());
            }
        }
        HubWindowMemory::new(self.organization.clone(), sessions)
    }

    fn save(&mut self, cx: &mut App) {
        let memory = self.memory();
        if memory != self.kept {
            self.kept = memory.clone();
            self.keeper.keep(&self.hub, memory, cx);
        }
    }
}

#[cfg(test)]
#[path = "remembered_tests.rs"]
mod tests;
