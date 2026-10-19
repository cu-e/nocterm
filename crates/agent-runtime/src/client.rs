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
    BridgeCall, BridgeRejection, PermissionResponder,
    connection::ManagedCommandsHandle,
    history::source::{ManagedAgentInfo, ManagedNotice, ManagedPermission},
    model::ids::SessionId,
};
use std::{path::PathBuf, rc::Rc};

/// What the runtime tells a chat.
pub enum SessionEvent {
    /// A connection is being started for the chat; the lease holds it.
    Leased(SessionLease),
    /// The agent process of connection `connection` is up: open or load
    /// the session.
    Connected {
        connection: u64,
        commands: ManagedCommandsHandle,
        info: ManagedAgentInfo,
        workdir: PathBuf,
    },
    /// An update for the chat's connection; it may belong to another session.
    Update(ManagedNotice),
    /// The agent asks to use a tool. Dropping `respond` answers `Cancelled`.
    Permission {
        request: ManagedPermission,
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
    pub chat_id: String,
    /// Restored chat directory, resolved before process and sandbox launch.
    pub workdir: Option<PathBuf>,
    pub agent_id: String,
    pub session: Option<SessionId>,
    /// Whether the session is doing work that closing it would interrupt.
    pub busy: bool,
    /// Whether the chat holds a connection.
    pub leased: bool,
    /// Whether the chat waits for a connection.
    pub activation_pending: bool,
    /// Whether the chat's previous session is still closing.
    pub closing: bool,
    /// Whether a panel shows the chat; its idle connection is kept.
    pub shown: bool,
}

/// A failed immutable capture names the original semantic revision explicitly.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{error}")]
pub struct CaptureError {
    revision: u64,
    #[source]
    error: super::AgentPersistenceError,
}
impl CaptureError {
    pub fn history(revision: u64, error: nocterm_ai::history::HistoryCodecError) -> Self {
        Self {
            revision,
            error: nocterm_store::StoreError::backend(error).into(),
        }
    }
    pub fn persistence(revision: u64, error: super::AgentPersistenceError) -> Self {
        Self { revision, error }
    }
    pub fn into_failure(self) -> (u64, super::AgentPersistenceError) {
        (self.revision, self.error)
    }
}

/// A chat as the runtime sees it. Calls on a closed chat do nothing.
pub trait SessionClient {
    fn id(&self) -> EntityId;
    /// Whether the chat is still open.
    fn alive(&self) -> bool;
    fn emit(&self, event: SessionEvent, cx: &mut App);
    fn state(&self, cx: &App) -> Option<ClientState>;
    /// Prepare copied launch facts using the current document's original Source
    /// capability before scheduling native startup.
    fn prepare_startup(
        &self,
        request: nocterm_ai::connection::ManagedStartupRequest<'_>,
        cx: &mut App,
    ) -> Result<nocterm_ai::connection::ManagedStartup, nocterm_ai::history::source::SourceError>;
    /// Admit original document roles synchronously while the view still exists.
    /// The supplied service capability preserves shutdown capture admission.
    fn capture_into(
        &self,
        admission: &super::CaptureAdmission,
        cx: &mut App,
    ) -> Result<Option<u64>, CaptureError>;
}

pub type Client = Rc<dyn SessionClient>;
