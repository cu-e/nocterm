//! Typed reconnect/open orchestration failures become tool results only at delivery.
use nocterm_workspace::TerminalError;
#[derive(Debug, thiserror::Error)]
pub(super) enum ConnectionWaitError {
    #[error("The server was detached before it connected.")]
    Detached,
    #[error("{0}")]
    Terminal(#[from] TerminalError),
    #[error("The workspace is unavailable.")]
    WorkspaceClosed,
    #[error("Could not open a session for this server.")]
    OpenFailed,
    #[error("The chat stopped before the server connected.")]
    Stopped,
    #[error("The session was closed before it connected.")]
    SessionClosed,
    #[error("The chat was closed.")]
    ChatClosed,
    #[error("Could not connect to the server.")]
    ConnectFailed,
    #[error("Connecting to the server timed out.")]
    Timeout,
    #[error(
        "The server is waiting for the user to sign in; ask them to finish signing in, then call list_terminals."
    )]
    VisiblePromptTimeout,
    #[error(
        "The server is waiting for the user to sign in from the chat or unlock the vault; ask them to, then call open_terminal again."
    )]
    HiddenPromptTimeout,
}
impl ConnectionWaitError {
    pub(super) fn into_failure(self) -> nocterm_ai::ToolFailure {
        match self {
            Self::WorkspaceClosed
            | Self::OpenFailed
            | Self::ConnectFailed
            | Self::Timeout
            | Self::Terminal(_) => super::tools::unreachable(&self.to_string()),
            _ => self.to_string().into(),
        }
    }
}
