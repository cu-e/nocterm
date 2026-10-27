//! What the window of one hub reads and does on the hub: its account, nodes and sessions,
//! shared by every window of that hub.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, EventEmitter, Task};
use nocterm_hubs_model::{
    Account, HubError, HubId, HubSessionId, HubSessionSummary, HubSessions, HubTerminalTarget,
    NodeId, NodeSummary, OrgId,
};
use nocterm_session::Transport;

/// How often the lists are read again while somebody watches them: the hub has no push for
/// the list of sessions.
const POLL: Duration = Duration::from_secs(5);

/// The lists of a hub window, as last read.
#[derive(Clone, Debug)]
pub enum Load {
    /// Nothing was asked yet.
    Idle,
    /// The first read is under way.
    Loading,
    Loaded,
    /// The last read failed; the lists keep what was read before.
    Failed(HubError),
}

/// What happens to a hub window's lists.
#[derive(Clone, Debug)]
pub enum HubWindowEvent {
    Changed,
    /// A call failed; the registry decides what it says about the hub's status.
    Failed(HubError),
}

/// Keeps the lists of a hub window fresh while it is alive.
#[must_use = "the lists stop refreshing when the watch is dropped"]
pub struct Watch {
    watchers: Rc<Cell<usize>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.watchers.set(self.watchers.get().saturating_sub(1));
    }
}

/// The hub's account, nodes and sessions for one organization, and the transports that
/// attach tabs to its sessions.
pub struct HubWindow {
    hub: HubId,
    sessions: Arc<dyn HubSessions>,
    account: Option<Account>,
    org: Option<OrgId>,
    nodes: Vec<NodeSummary>,
    list: Vec<HubSessionSummary>,
    load: Load,
    reading: Reading,
    polling: Polling,
}

/// The read in flight and whether another was asked for meanwhile.
#[derive(Default)]
struct Reading {
    task: Option<Task<()>>,
    again: bool,
}

#[derive(Default)]
struct Polling {
    watchers: Rc<Cell<usize>>,
    task: Option<Task<()>>,
}

impl EventEmitter<HubWindowEvent> for HubWindow {}

impl HubWindow {
    pub(crate) fn new(hub: HubId, sessions: Arc<dyn HubSessions>) -> Self {
        Self {
            hub,
            sessions,
            account: None,
            org: None,
            nodes: Vec::new(),
            list: Vec::new(),
            load: Load::Idle,
            reading: Reading::default(),
            polling: Polling::default(),
        }
    }

    pub fn hub(&self) -> &HubId {
        &self.hub
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// The organization whose nodes and sessions are shown.
    pub fn org(&self) -> Option<&OrgId> {
        self.org.as_ref()
    }

    pub fn nodes(&self) -> &[NodeSummary] {
        &self.nodes
    }

    /// Sessions that have not ended.
    pub fn sessions(&self) -> &[HubSessionSummary] {
        &self.list
    }

    pub fn load(&self) -> &Load {
        &self.load
    }

    /// Whether the session was started by the account this device is signed in as.
    pub fn is_mine(&self, session: &HubSessionSummary) -> bool {
        self.account
            .as_ref()
            .is_some_and(|account| session.is_owned_by(account))
    }

    /// Shows another organization of the account.
    pub fn select_org(&mut self, org: OrgId, cx: &mut Context<Self>) {
        if self.org.as_ref() == Some(&org) {
            return;
        }
        self.org = Some(org);
        self.nodes.clear();
        self.list.clear();
        self.refresh(cx);
        cx.emit(HubWindowEvent::Changed);
        cx.notify();
    }

    /// Reads the lists again. A read already under way is not doubled; it is repeated once
    /// it finishes, so the newest state wins.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.reading.task.is_some() {
            self.reading.again = true;
            return;
        }
        if matches!(self.load, Load::Idle) {
            self.load = Load::Loading;
            cx.emit(HubWindowEvent::Changed);
            cx.notify();
        }
        let sessions = Arc::clone(&self.sessions);
        let hub = self.hub.clone();
        let asked = self.org.clone();
        let known = (self.account.clone(), asked.clone());
        self.reading.task = Some(cx.spawn(async move |this, cx| {
            let read = read(&*sessions, &hub, known).await;
            this.update(cx, |window, cx| window.finish_read(asked, read, cx))
                .ok();
        }));
    }

    /// Takes what a read found. `asked` is the organization the read began with: when another
    /// was chosen meanwhile, the lists of the old one are not shown and the read that
    /// `select_org` asked for follows.
    fn finish_read(
        &mut self,
        asked: Option<OrgId>,
        read: Result<Read, HubError>,
        cx: &mut Context<Self>,
    ) {
        self.reading.task = None;
        match read {
            Ok(read) if self.org != asked => self.account = Some(read.account),
            Ok(read) => {
                self.org = read.org;
                self.account = Some(read.account);
                self.nodes = read.nodes;
                self.list = read.sessions;
                self.load = Load::Loaded;
            }
            Err(error) => {
                self.load = Load::Failed(error.clone());
                cx.emit(HubWindowEvent::Failed(error));
            }
        }
        cx.emit(HubWindowEvent::Changed);
        cx.notify();
        if std::mem::take(&mut self.reading.again) {
            self.refresh(cx);
        }
    }

    /// Terminates the process of a session on its node, then reads the lists again.
    pub fn end(
        &mut self,
        session: HubSessionId,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), HubError>> {
        let sessions = Arc::clone(&self.sessions);
        let hub = self.hub.clone();
        cx.spawn(async move |this, cx| {
            let ended = sessions.end(&hub, &session).await;
            this.update(cx, |window, cx| {
                if let Err(error) = &ended {
                    cx.emit(HubWindowEvent::Failed(error.clone()));
                }
                window.refresh(cx);
            })
            .ok();
            ended
        })
    }

    /// The transport of a tab attached to a session. Closing the tab detaches; the process
    /// keeps running on its node.
    pub fn terminal(&self, target: HubTerminalTarget) -> Arc<dyn Transport> {
        self.sessions.terminal(&self.hub, target)
    }

    /// The target of a new session on `node`, or on a node the hub picks. `None` until the
    /// account and its organization are known.
    pub fn new_session(&self, node: Option<NodeId>, title: &str) -> Option<HubTerminalTarget> {
        Some(HubTerminalTarget::New {
            org: self.org.clone()?,
            node,
            title: title.to_owned(),
        })
    }

    /// Reads the lists every few seconds for as long as the returned guard lives.
    pub fn watch(&mut self, cx: &mut Context<Self>) -> Watch {
        self.polling.watchers.set(self.polling.watchers.get() + 1);
        if self.polling.task.is_none() {
            self.polling.task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(POLL).await;
                    let watched = this.update(cx, |window, cx| {
                        let watched = window.polling.watchers.get() > 0;
                        if watched {
                            window.refresh(cx);
                        } else {
                            window.polling.task = None;
                        }
                        watched
                    });
                    if !matches!(watched, Ok(true)) {
                        break;
                    }
                }
            }));
        }
        Watch {
            watchers: Rc::clone(&self.polling.watchers),
        }
    }
}

struct Read {
    account: Account,
    org: Option<OrgId>,
    nodes: Vec<NodeSummary>,
    sessions: Vec<HubSessionSummary>,
}

/// The account if it is not known yet, then the nodes and sessions of the organization.
async fn read(
    sessions: &dyn HubSessions,
    hub: &HubId,
    (account, org): (Option<Account>, Option<OrgId>),
) -> Result<Read, HubError> {
    let account = match account {
        Some(account) => account,
        None => sessions.account(hub).await?,
    };
    let org = org.or_else(|| first_org(&account));
    let (nodes, list) = match &org {
        Some(org) => {
            let (nodes, list) =
                futures::join!(sessions.nodes(hub, org), sessions.sessions(hub, org));
            (nodes?, list?)
        }
        None => (Vec::new(), Vec::new()),
    };
    Ok(Read {
        account,
        org,
        nodes,
        sessions: list,
    })
}

/// The organization shown first: the first by name.
fn first_org(account: &Account) -> Option<OrgId> {
    account
        .orgs
        .iter()
        .min_by(|a, b| a.name.cmp(&b.name))
        .map(|org| org.id.clone())
}

#[cfg(test)]
mod tests;
