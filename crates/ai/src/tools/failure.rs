//! Failures of terminal tool calls, as typed rejections and as agent-facing text.

/// Why a terminal tool call was rejected before execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminalCallError {
    #[error("Invalid terminal or server id")]
    InvalidTarget,
    #[error("Read limit must be 1–2000 lines")]
    ReadLimit,
    #[error("Input too large or contains NUL")]
    InputRejected,
    #[error("Command must be a single nonempty line, at most 16 KiB")]
    CommandRejected,
    #[error("Invalid timeout or idle interval")]
    RunTiming,
    #[error("Invalid program, arguments or stdin size")]
    ProgramRejected,
    #[error("Invalid execution timeout or yield interval")]
    ExecTiming,
    #[error("Invalid command id or yield interval")]
    ReadCommandRejected,
    #[error("Invalid command id")]
    InvalidCommandId,
    #[error(
        "Transfer needs 1–64 absolute source paths and one destination folder (each at most 4096 bytes without control characters, sources at most 64 KiB in total) and a yield of at most 10000 ms"
    )]
    TransferRejected,
    #[error("Invalid transfer id or yield interval")]
    ReadTransferRejected,
    #[error("Invalid transfer id")]
    InvalidTransferId,
}

/// Agent-facing text of a failed tool call.
///
/// This is MCP and transcript payload text: it is sent to the agent, shown in
/// the transcript and redacted only at the result boundary (`mcp::tool_result`,
/// `ToolDisplay::finish`). `Display` yields the raw, unredacted text, so never
/// log or show it outside those boundaries. Typed rejections convert into it
/// with `From`; free-form messages convert from text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ToolFailure {
    message: String,
}

impl ToolFailure {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<String> for ToolFailure {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for ToolFailure {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

impl From<TerminalCallError> for ToolFailure {
    fn from(error: TerminalCallError) -> Self {
        error.to_string().into()
    }
}

/// Serialization failed without discarding its original concrete cause.
#[derive(Clone, thiserror::Error)]
#[error("Terminal tool arguments could not be encoded")]
pub struct ToolArgumentsError(#[source] nocterm_core::SharedError<serde_json::Error>);
impl From<serde_json::Error> for ToolArgumentsError {
    fn from(error: serde_json::Error) -> Self {
        Self(nocterm_core::SharedError::from_error(error))
    }
}
impl std::fmt::Debug for ToolArgumentsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ToolArgumentsError")
    }
}
