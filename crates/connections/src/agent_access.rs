//! The port through which a hub window shows and changes whether the hub's agents may work on a
//! saved server (ADR 0148): the editor's Agent access switch and the state on the server's row.
//! The composition root implements it over the hub agents service.
use gpui_kit::{App, SharedString, Subscription, Task};
use nocterm_connections_model::profiles::Profile;

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

impl AgentServer {
    /// The saved connection `profile`, as the hub's agent servers are matched against it.
    pub(crate) fn of(profile: &Profile) -> Self {
        Self {
            connection: profile.id.to_string(),
            name: profile.name.clone(),
            host: profile.target.host.clone(),
            port: profile.target.port,
            user: profile.target.user.clone(),
        }
    }
}

/// What the hub's agents last found on a server they may sign in to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAccessState {
    /// Recorded on the hub; agents have not signed in yet.
    Unchecked,
    /// Agents sign in.
    Ready,
    /// The server refuses the agent key.
    NoAccess,
    /// The server presents another host key than the recorded one.
    HostKeyChanged,
    /// The server could not be reached.
    Unreachable,
    /// The agent key was replaced after its line was written: the server needs the new one.
    Reconnect,
}

impl AgentAccessState {
    /// The short text of the server's row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unchecked => "Agents",
            Self::Ready => "Agents ✓",
            Self::NoAccess => "No access",
            Self::HostKeyChanged => "Host key changed",
            Self::Unreachable => "Unreachable",
            Self::Reconnect => "Reconnect",
        }
    }

    /// Whether nothing needs the user's attention.
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready | Self::Unchecked)
    }
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
    /// The state of `server` among the hub's agent servers, as last listed; none while the hub
    /// does not record it.
    fn state(&self, server: &AgentServer, cx: &App) -> Option<AgentAccessState>;
    /// Whether a session to `server` is open, so the agent key's line can be written through it.
    fn session_open(&self, server: &AgentServer, cx: &App) -> bool;
    /// Grants or withdraws access; the task ends once the hub holds the new state.
    fn set(
        &self,
        server: AgentServer,
        granted: bool,
        cx: &mut App,
    ) -> Task<Result<(), AgentAccessRefused>>;
    /// Calls `changed` whenever the hub's agent servers may have changed.
    fn observe(&self, changed: Box<dyn Fn(&mut App)>, cx: &mut App) -> Subscription;
}
