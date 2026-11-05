//! The servers a remote conversation's agent may sign in to (ADR 0139): what a window shows of
//! a chat's binding, and the captured port through which the person binds, unbinds and picks
//! the server prompts run on. Neutral display values; the source decides what is allowed.
use super::ConversationError;
use gpui::{App, Task};
use std::time::Duration;

/// Why binding servers, or naming one for prompts, was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServerProblem {
    #[error(
        "The account has no agent CA yet. Allow a server on the hub's Agent access page first."
    )]
    NoCa,
    #[error("A chosen server is no longer trusted by the account's CA.")]
    Untrusted,
    #[error("Connect to the server once so its port is known, then bind it.")]
    UnknownPort,
    #[error("Choose between one and sixteen servers.")]
    Servers,
    #[error("A binding lasts at most thirty days.")]
    Window,
    #[error("The chat is not bound to servers.")]
    NotBound,
    #[error("The server cannot run prompts now.")]
    NotSelectable,
}

/// Where one bound server stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundServerState {
    /// Waiting for an unlocked device of the account to certify the agent's key.
    Pending,
    /// The agent may sign in until this time (milliseconds since the Unix epoch).
    ActiveUntil(i64),
    /// The agent's runner cannot reach the server.
    Unreachable,
    /// The runner refused the binding.
    Refused,
    /// The binding ended.
    Expired,
    /// The binding was revoked.
    Revoked,
}

/// One server of the conversation's binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundServerView {
    /// Stable within the conversation; names the server to [`ConversationServers`].
    pub id: String,
    /// `user@host:port`.
    pub label: String,
    pub state: BoundServerState,
    /// A prompt may run on it now.
    pub selectable: bool,
}

/// A server the conversation can be bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerOption {
    pub id: String,
    /// `user@host`.
    pub label: String,
    /// The port is known, so it can be bound now.
    pub bindable: bool,
    /// It still needs a CA update (its next connection gives it its own line).
    pub needs_ca_update: bool,
}

/// The binding of one conversation, captured from its source.
pub trait ConversationServers {
    /// The bound servers, in the binding's order; empty when it is not bound.
    fn bound(&self, cx: &App) -> Vec<BoundServerView>;
    /// When the binding ends.
    fn expires_at_ms(&self, cx: &App) -> Option<i64>;
    /// The server the next prompts run on; `None` runs them at the agent's default destination.
    fn target(&self, cx: &App) -> Option<String>;
    /// Picks the server the next prompts run on; only a selectable bound server, or `None`.
    fn set_target(&self, id: Option<String>, cx: &mut App) -> Result<(), ConversationError>;
    /// The servers the conversation can be bound to.
    fn options(&self, cx: &mut App) -> Task<Result<Vec<ServerOption>, ConversationError>>;
    /// Binds the conversation to `ids` for `window`, replacing its binding.
    fn bind(
        &self,
        ids: Vec<String>,
        window: Duration,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>>;
    /// Revokes the binding.
    fn unbind(&self, cx: &mut App) -> Task<Result<(), ConversationError>>;
}
