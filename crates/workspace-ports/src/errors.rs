//! Typed refusals at the terminal seams.
use nocterm_session::TextEncodingError;

/// An optional operation refused by a terminal provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminalCapability {
    #[error("This terminal does not support reading text.")]
    TextRead,
    #[error("This terminal does not support text input.")]
    Input,
    #[error("This terminal does not support sign-in prompts.")]
    SignIn,
    #[error("This terminal does not support structured execution.")]
    StructuredExecution,
    #[error("This session does not support structured execution.")]
    SessionExecution,
    #[error("This terminal does not support owned live commands.")]
    LiveCommands,
    #[error("This terminal does not support snippets.")]
    Snippets,
    #[error("This terminal cannot reconnect.")]
    Reconnect,
}

/// A terminal operation refused before submission, or rejected by its queue.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("The terminal connection changed before this operation.")]
    StaleSession,
    #[error("The sign-in prompt changed before this answer.")]
    StalePrompt,
    #[error("Terminal was closed.")]
    Gone,
    #[error("Terminal is unavailable or waiting for authentication.")]
    Unavailable,
    #[error(transparent)]
    Unsupported(TerminalCapability),
    #[error("Only a remote session can reconnect.")]
    OnlyRemoteReconnect,
    #[error("The session no longer asks to sign in.")]
    SignInNoLongerRequested,
    #[error("Terminal input exceeds 16 KiB.")]
    InputTooLarge,
    #[error("Run one command without control characters.")]
    CommandControlCharacters,
    #[error("The terminal is busy, has unfinished input, or is in an alternate screen.")]
    CommandPromptUnavailable,
    #[error("Another chat is observing a command in this terminal.")]
    LiveCommandOwned,
    #[error("Leave the alternate screen before running a snippet.")]
    SnippetAlternateScreen,
    #[error("Snippet content is empty.")]
    EmptySnippet,
    #[error("Snippet content exceeds 256 KiB.")]
    SnippetTooLarge,
    #[error("Snippet content contains NUL or Escape control characters.")]
    SnippetControlCharacters,
    #[error("Terminal input queue rejected the input.")]
    InputQueueRejected,
    #[error("Terminal input queue rejected the paste. Wait and retry.")]
    PasteQueueRejected,
    #[error("{0}")]
    Encoding(#[from] TextEncodingError),
    #[error("Paste could not be encoded as text.")]
    PasteTextEncoding(#[from] std::str::Utf8Error),
}
