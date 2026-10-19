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
