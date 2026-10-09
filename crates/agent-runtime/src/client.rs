//! The port through which the runtime reaches the chats that use it.
//!
//! The runtime knows no chat type. A chat registers a [`SessionClient`] and
//! changes its own state when it receives a [`SessionEvent`]. Events are
//! delivered synchronously, in the order the runtime emits them.
use super::SessionLease;
use gpui::{App, EntityId};
use nocterm_ai::{AgentCommands, AgentInfo, BridgeCall, PermissionResponder, acp};
use std::{path::PathBuf, rc::Rc, sync::Arc};

/// What the runtime tells a chat.
pub enum SessionEvent {
    /// A connection is being started for the chat; the lease holds it.
    Leased(SessionLease),
    /// The agent process is up: open or load the session.
    Connected {
        commands: Arc<dyn AgentCommands>,
        info: Box<AgentInfo>,
        workdir: PathBuf,
    },
    /// An update for the chat's connection; it may belong to another session.
    Update(Rc<acp::SessionNotification>),
    /// The agent asks to use a tool. Dropping `respond` answers `Cancelled`.
    Permission {
        request: Box<acp::RequestPermissionRequest>,
        respond: PermissionResponder,
    },
    /// A call to one of the chat's terminal tools.
    Tool(BridgeCall),
    /// The connection ended with `0`.
    Stopped(String),
    /// The connection could not be started.
    Failed(String),
    /// The chat's agent was removed from the settings before it started.
    AgentRemoved,
    /// The connection was closed to free a slot; the conversation stays.
    Idle,
    /// The agent process may still be running after its connection closed.
    CleanupUnconfirmed(String),
    /// The previous session's close finished; a new one may start.
    SessionClosed,
    /// The chat's snapshot at `revision` was written.
    Saved { revision: u64 },
    /// The chat's last snapshot could not be written.
    SaveFailed(String),
    /// The permission settings changed.
    PolicyChanged,
    /// The application is quitting: release everything the agent holds.
    Shutdown,
}

/// What the runtime reads about a chat to schedule its connection.
pub struct ClientState {
    pub chat_id: String,
    pub agent_id: String,
    pub session: Option<acp::SessionId>,
    /// Whether the session is doing work that closing it would interrupt.
    pub busy: bool,
    /// Whether the chat holds a connection.
    pub leased: bool,
    /// Whether the chat waits for a connection.
    pub activation_pending: bool,
    /// Whether the chat's previous session is still closing.
    pub closing: bool,
}

/// A chat as the runtime sees it. Calls on a closed chat do nothing.
pub trait SessionClient {
    fn id(&self) -> EntityId;
    /// Whether the chat is still open.
    fn alive(&self) -> bool;
    fn emit(&self, event: SessionEvent, cx: &mut App);
    fn state(&self, cx: &App) -> Option<ClientState>;
    /// The chat's unsaved changes, without changing it.
    fn snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>>;
    /// The chat's unsaved changes and the revision they become.
    fn capture(&self, cx: &mut App) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)>;
}

pub type Client = Rc<dyn SessionClient>;
