//! What a hub window keeps of its server across a restart: the organization it shows and the
//! sessions its tabs are attached to.
//!
//! The owner is told when a tab opens, learns the id of a session it started, closes, or when
//! the organization changes, and hands the whole memory to the [`HubMemoryKeeper`] when it
//! differs from the last one it gave. A session is remembered with its own organization, since
//! tabs of several organizations can be open at once. After a restart it holds the remembered
//! sessions until the hub lists the sessions of their organization, then names the ones that
//! can be attached again.

use std::rc::Rc;

use gpui_kit::{App, Context, Subscription};
use nocterm_hubs_model::{
    HubId, HubSessionId, HubSessionState, HubSessionSummary, HubWindowMemory, OrgId,
    RememberedSession,
};

use crate::ports::HubMemoryKeeper;

/// One open tab, as far as the memory is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TabId(u64);

struct Tab {
    id: TabId,
    /// The organization the session of the tab belongs to.
    org: OrgId,
    /// The session the tab is attached to; not known for a new session until the hub made it.
    session: Option<HubSessionId>,
}

/// The memory of one hub window.
pub(crate) struct Remembered {
    hub: HubId,
    keeper: Rc<dyn HubMemoryKeeper>,
    organization: Option<OrgId>,
    /// Sessions remembered from the previous run that no tab is attached to yet.
    pending: Vec<RememberedSession>,
    tabs: Vec<Tab>,
    next_tab: u64,
    /// Sessions that were found but whose tab could not open; they are kept for the next run.
    unattached: Vec<RememberedSession>,
    /// The organizations whose sessions are being read to attach pending ones.
    fetching: Vec<OrgId>,
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
            unattached: Vec::new(),
            fetching: Vec::new(),
            kept: restored,
            _watch: None,
        }
    }

    pub(crate) fn watching(&mut self, watch: Subscription) {
        self._watch = Some(watch);
    }

    /// A tab opened on `session` of `org`, if that is known already. A session that was
    /// waiting to be attached again is attached now.
    pub(crate) fn tab_opened(
        &mut self,
        org: OrgId,
        session: Option<HubSessionId>,
        cx: &mut Context<Self>,
    ) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        if let Some(session) = &session {
            self.pending.retain(|waiting| waiting.id() != session);
        }
        self.tabs.push(Tab { id, org, session });
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

    /// The organizations that remembered sessions wait for, in the order they were remembered.
    pub(crate) fn waiting_orgs(&self) -> Vec<OrgId> {
        let mut orgs: Vec<OrgId> = Vec::new();
        for session in &self.pending {
            if !orgs.contains(session.org()) {
                orgs.push(session.org().clone());
            }
        }
        orgs
    }

    /// The sessions remembered for `org` that `listed`, the sessions the hub runs in it, still
    /// has and that no tab shows; the others ended meanwhile and are forgotten. Called once the
    /// hub has listed the organization: what it returns is attached again, and a second call
    /// finds nothing. An empty `listed` forgets them all, as for an organization the account
    /// left.
    pub(crate) fn reattachable(
        &mut self,
        org: &OrgId,
        listed: &[HubSessionSummary],
        cx: &mut Context<Self>,
    ) -> Vec<HubSessionSummary> {
        let (asked, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|session| session.org() == org);
        self.pending = rest;
        let open = |session: &HubSessionId| {
            self.tabs
                .iter()
                .any(|tab| tab.session.as_ref() == Some(session))
        };
        let found = asked
            .iter()
            .filter(|session| !open(session.id()))
            .filter_map(|session| {
                listed.iter().find(|candidate| {
                    candidate.id == *session.id()
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

    /// Starts reading the sessions of `org`; false when that is under way already.
    pub(crate) fn begin_fetch(&mut self, org: &OrgId) -> bool {
        if self.fetching.contains(org) {
            return false;
        }
        self.fetching.push(org.clone());
        true
    }

    /// The read of the sessions of `org` ended, with or without an answer.
    pub(crate) fn end_fetch(&mut self, org: &OrgId) {
        self.fetching.retain(|reading| reading != org);
    }

    /// The tab of a found session could not open. The session stays remembered for the next
    /// run; it is not tried again in this one.
    pub(crate) fn tab_failed(&mut self, session: &HubSessionSummary, cx: &mut Context<Self>) {
        self.unattached.push(RememberedSession::new(
            session.org.clone(),
            session.id.clone(),
        ));
        self.save(cx);
    }

    /// The sessions of the tabs in the order they opened, after those still waiting.
    fn memory(&self) -> HubWindowMemory {
        let mut sessions = self.pending.clone();
        sessions.extend(self.unattached.iter().cloned());
        for tab in &self.tabs {
            let Some(id) = &tab.session else {
                continue;
            };
            if !sessions.iter().any(|known| known.id() == id) {
                sessions.push(RememberedSession::new(tab.org.clone(), id.clone()));
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
