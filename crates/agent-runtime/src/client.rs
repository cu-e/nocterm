//! The port through which the runtime reaches the chats that use it.
//!
//! The runtime knows no chat type. A chat registers a [`SessionClient`] and
//! changes its own state when it receives a [`SessionEvent`]. Events are
//! delivered synchronously, in the order the runtime emits them. A chat
//! releases its connection by dropping its lease, which the runtime learns
//! of a moment later; events naming a connection let the chat ignore those
//! of one it no longer holds.
use super::SessionLease;
use gpui::{App, EntityId};
use nocterm_ai::{
    AgentCommands, AgentInfo, BridgeCall, BridgeRejection, PermissionResponder,
    model::{auth::ProviderPermission, ids::SessionId, session::SessionNotice},
};
use std::{path::PathBuf, rc::Rc, sync::Arc};

/// What the runtime tells a chat.
pub enum SessionEvent {
    /// A connection is being started for the chat; the lease holds it.
    Leased(SessionLease),
    /// The agent process of connection `connection` is up: open or load
    /// the session.
    Connected {
        connection: u64,
        commands: Arc<dyn AgentCommands>,
        info: Box<AgentInfo>,
        workdir: PathBuf,
    },
    /// An update for the chat's connection; it may belong to another session.
    Update(Rc<SessionNotice>),
    /// The agent asks to use a tool. Dropping `respond` answers `Cancelled`.
    Permission {
        request: Box<ProviderPermission>,
        respond: PermissionResponder,
    },
    /// A call to one of the chat's terminal tools.
    Tool(BridgeCall),
    ToolRejected(BridgeRejection),
    /// Connection `connection` ended with `message`.
    Stopped {
        connection: u64,
        message: String,
    },
    /// The connection could not be started.
    Failed(String),
    /// The chat's agent was removed from the settings before it started.
    AgentRemoved,
    /// Connection `connection` is closed to free a slot; the conversation
    /// stays.
    Idle {
        connection: u64,
    },
    /// The agent process may still be running after its connection closed.
    CleanupUnconfirmed(String),
    /// The previous session's close finished; a new one may start.
    SessionClosed,
    /// The shared writer accepted and owns this revision's immutable candidate.
    SaveAdmitted {
        revision: u64,
    },
    /// The chat's snapshot at `revision` was written.
    Saved {
        revision: u64,
    },
    /// The chat's last snapshot could not be written.
    SaveFailed {
        revision: u64,
        error: super::AgentPersistenceError,
    },
    /// The permission settings changed.
    PolicyChanged,
    /// The application is quitting: release everything the agent holds.
    Shutdown,
}

/// What the runtime reads about a chat to schedule its connection.
pub struct ClientState {
    origin: nocterm_ai::execution_origin::ExecutionOrigin,
    agent_id: String,
    session: Option<SessionId>,
    busy: bool,
    leased: bool,
    activation_pending: bool,
    closing: bool,
    shown: bool,
}
impl ClientState {
    pub fn new(
        origin: nocterm_ai::execution_origin::ExecutionOrigin,
        agent_id: String,
        session: Option<SessionId>,
        busy: bool,
        leased: bool,
        activation_pending: bool,
        closing: bool,
        shown: bool,
    ) -> Self {
        Self {
            origin,
            agent_id,
            session,
            busy,
            leased,
            activation_pending,
            closing,
            shown,
        }
    }
    pub fn origin(&self) -> &nocterm_ai::execution_origin::ExecutionOrigin {
        &self.origin
    }
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
    pub fn session(&self) -> &Option<SessionId> {
        &self.session
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn leased(&self) -> bool {
        self.leased
    }
    pub fn activation_pending(&self) -> bool {
        self.activation_pending
    }
    pub fn closing(&self) -> bool {
        self.closing
    }
    pub fn shown(&self) -> bool {
        self.shown
    }
}

/// Immutable resident payload observation; eviction safety remains with the chat.
#[derive(Clone, Copy)]
pub struct HistoryUsage {
    bytes: usize,
    updated: u64,
    shown: bool,
}
impl HistoryUsage {
    pub fn new(bytes: usize, updated: u64, shown: bool) -> Self {
        Self {
            bytes,
            updated,
            shown,
        }
    }
    pub(crate) fn bytes(self) -> usize {
        self.bytes
    }
    pub(crate) fn updated(self) -> u64 {
        self.updated
    }
    pub(crate) fn shown(self) -> bool {
        self.shown
    }
}
/// A chat as the runtime sees it. Calls on a closed chat do nothing.
pub trait SessionClient {
    fn id(&self) -> EntityId;
    /// Whether the chat is still open.
    fn alive(&self) -> bool;
    fn emit(&self, event: SessionEvent, cx: &mut App);
    fn state(&self, cx: &App) -> Option<ClientState>;
    /// Resident history only; callers must exclude a currently borrowed requester.
    fn history_usage(&self, cx: &App) -> Option<HistoryUsage>;
    /// Evict only through the chat's durability, lease and dirty-input gates.
    fn evict_history(&self, cx: &mut App) -> bool;
    /// The chat's unsaved changes, without changing it.
    fn snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>>;
    /// The chat's unsaved changes and the revision they become.
    fn capture(&self, cx: &mut App) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)>;
}

pub type Client = Rc<dyn SessionClient>;
