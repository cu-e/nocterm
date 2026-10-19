use super::{
    commands::AvailableCommand,
    content::Content,
    controls::{Change, ConfigOption, ConfigSetting, Modes},
    extensions::Extensions,
    ids::{ConfigId, ModeId, SessionId},
    plan::Plan,
    tools::{ToolPatch, ToolRecord},
    usage::{ContextUsage, TokenTotals},
};
use std::path::PathBuf;

/// A request to start a new session.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionOpen {
    pub working_directory: PathBuf,
    pub endpoints: Vec<ExternalToolEndpoint>,
}

/// A request to reopen, or fork, a session the agent held before.
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreSession {
    pub session: SessionId,
    pub working_directory: PathBuf,
    pub endpoints: Vec<ExternalToolEndpoint>,
    /// Open a new session that starts with this one's history instead.
    pub fork: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionOpened {
    pub session: SessionId,
    pub modes: Option<Modes>,
    pub configuration: Option<Vec<ConfigOption>>,
}
impl SessionOpened {
    pub fn new(session: impl Into<SessionId>) -> Self {
        Self {
            session: session.into(),
            modes: None,
            configuration: None,
        }
    }
}

/// How the agent starts a local tool server for the session.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalToolEndpoint {
    pub name: String,
    pub launch: crate::BridgeLaunch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromptRequest {
    pub session: SessionId,
    pub content: Vec<Content>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromptOutcome {
    pub stop_reason: StopReason,
    pub usage: Option<TokenTotals>,
}
impl PromptOutcome {
    pub fn new(stop_reason: StopReason) -> Self {
        Self {
            stop_reason,
            usage: None,
        }
    }
}

/// Variant names are shown to the user through `Debug`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetMode {
    pub session: SessionId,
    pub mode: ModeId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetConfigOption {
    pub session: SessionId,
    pub config: ConfigId,
    pub setting: ConfigSetting,
}

/// One update for a session, as the agent reported it.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionNotice {
    pub session: SessionId,
    pub update: SessionUpdate,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionUpdate {
    UserMessageChunk(Content),
    AgentMessageChunk(Content),
    AgentThoughtChunk(Content),
    ToolCall(Box<ToolRecord>),
    ToolCallUpdate(Box<ToolPatch>),
    Plan(Plan),
    AvailableCommandsUpdate(Vec<AvailableCommand>),
    CurrentModeUpdate(ModeId),
    ConfigOptionUpdate(Vec<ConfigOption>),
    SessionInfoUpdate(SessionInfo),
    UsageUpdate(ContextUsage),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionInfo {
    pub title: Change<String>,
    pub meta: Option<Extensions>,
}

/// An error the agent reported. Data the agent attached is not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: i32,
    pub message: String,
}
impl RemoteError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const AUTH_REQUIRED: i32 = -32000;
}
impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            f.write_str(&self.message)
        }
    }
}

/// What the agent offers beyond the commands every agent answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentCapabilitiesSummary {
    pub image_prompts: bool,
}
