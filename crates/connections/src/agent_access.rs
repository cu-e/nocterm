//! The port through which the connection editor of a hub window lets the hub's agents work on
//! a saved server (ADR 0148). The composition root implements it over the hub agents service.
use gpui_kit::{App, SharedString, Task};

/// The saved server whose agent access is shown and changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentServer {
    /// The id of the saved connection.
    pub connection: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// Why agent access did not change, in words the editor shows.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{reason}")]
pub struct AgentAccessRefused {
    reason: SharedString,
}

impl AgentAccessRefused {
    pub fn new(reason: impl Into<SharedString>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &SharedString {
        &self.reason
    }
}

/// Whether the agents of the window's hub may sign in to a saved server, and the switch for it.
pub trait ServerAgentAccess: Send + Sync {
    /// Whether the hub records `server` among its agent servers now.
    fn granted(&self, server: &AgentServer, cx: &App) -> bool;
    /// Grants or withdraws access; the task ends once the hub holds the new state.
    fn set(
        &self,
        server: AgentServer,
        granted: bool,
        cx: &mut App,
    ) -> Task<Result<(), AgentAccessRefused>>;
}
