use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub const MAX_READ_LINES: usize = 2000;
pub const MAX_READ_BYTES: usize = 64 * 1024;
pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_MCP_LINE: usize = 1024 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadTerminal {
    pub terminal_id: String,
    pub lines: Option<usize>,
    pub since: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    pub terminal_id: String,
    pub text: String,
    #[serde(default)]
    pub press_enter: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCommand {
    pub terminal_id: String,
    pub command: String,
    pub timeout_ms: Option<u64>,
    pub idle_ms: Option<u64>,
}
/// Connects to an attached offline server without opening a tab.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenTerminal {
    /// An `offline_servers` entry of `list_terminals`.
    pub server_id: String,
}
#[derive(Clone, Debug)]
pub enum TerminalCall {
    ListTerminals,
    ReadTerminal(ReadTerminal),
    SendInput(SendInput),
    RunCommand(RunCommand),
    OpenTerminal(OpenTerminal),
}
impl TerminalCall {
    pub fn terminal_id(&self) -> Option<&str> {
        match self {
            Self::ListTerminals | Self::OpenTerminal(_) => None,
            Self::ReadTerminal(v) => Some(&v.terminal_id),
            Self::SendInput(v) => Some(&v.terminal_id),
            Self::RunCommand(v) => Some(&v.terminal_id),
        }
    }
    pub fn server_id(&self) -> Option<&str> {
        match self {
            Self::OpenTerminal(v) => Some(&v.server_id),
            _ => None,
        }
    }
    /// What an approval is granted for: a terminal, or a server to open.
    pub fn target(&self) -> Option<&str> {
        self.terminal_id().or_else(|| self.server_id())
    }
    /// Typing, running and connecting change something; reading does not.
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Self::SendInput(_) | Self::RunCommand(_) | Self::OpenTerminal(_)
        )
    }
    pub fn validate(&self) -> Result<(), String> {
        if self
            .target()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err("Invalid terminal or server id".into());
        }
        match self {
            Self::ReadTerminal(v) if v.lines.is_some_and(|n| n == 0 || n > MAX_READ_LINES) => {
                Err("Read limit must be 1–2000 lines".into())
            }
            Self::SendInput(v) if v.text.len() > MAX_INPUT_BYTES || v.text.contains('\0') => {
                Err("Input too large or contains NUL".into())
            }
            Self::RunCommand(v)
                if v.command.trim().is_empty()
                    || v.command.len() > MAX_INPUT_BYTES
                    || v.command.contains(['\0', '\n', '\r']) =>
            {
                Err("Command must be a single nonempty line, at most 16 KiB".into())
            }
            Self::RunCommand(v)
                if v.timeout_ms.is_some_and(|n| n == 0 || n > 300_000)
                    || v.idle_ms.is_some_and(|n| !(100..=30_000).contains(&n)) =>
            {
                Err("Invalid timeout or idle interval".into())
            }
            _ => Ok(()),
        }
    }
}
