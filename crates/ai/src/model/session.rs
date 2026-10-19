use super::{
    content::Content,
    controls::{ConfigOption, Modes},
    ids::SessionId,
    usage::TokenTotals,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq)]
pub struct SessionOpen {
    pub working_directory: PathBuf,
    pub endpoints: Vec<ExternalToolEndpoint>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreSession {
    pub session: SessionId,
    pub working_directory: PathBuf,
    pub endpoints: Vec<ExternalToolEndpoint>,
    pub fork: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SessionOpened {
    pub session: SessionId,
    pub modes: Option<Modes>,
    pub configuration: Option<Vec<ConfigOption>>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ExternalToolEndpoint {
    LocalCommand {
        name: String,
        launch: crate::BridgeLaunch,
    },
    Http {
        name: String,
        endpoint: String,
        headers: Vec<(String, String)>,
    },
    EventStream {
        name: String,
        endpoint: String,
        headers: Vec<(String, String)>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct PromptRequest {
    pub session: SessionId,
    pub content: Vec<Content>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PromptOutcome {
    pub ending: EndOfTurn,
    pub tokens: Option<TokenTotals>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndOfTurn {
    Finished,
    TokenLimit,
    RequestLimit,
    Refused,
    Cancelled,
    Other(String),
}

/// Semantic capabilities used by chat consumers, without provider lifecycle spelling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub content: ContentCapabilities,
    pub restore: bool,
    pub fork: bool,
    pub close: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_resources: bool,
}
impl EndOfTurn {
    /// Existing status labels are a presentation projection, not cancellation authority.
    pub fn display_label(&self) -> &str {
        match self {
            Self::Finished => "EndTurn",
            Self::TokenLimit => "MaxTokens",
            Self::RequestLimit => "MaxTurnRequests",
            Self::Refused => "Refusal",
            Self::Cancelled => "Cancelled",
            Self::Other(label) => label,
        }
    }
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ending_labels_preserve_display_without_granting_cancellation() {
        assert_eq!(EndOfTurn::Finished.display_label(), "EndTurn");
        assert!(EndOfTurn::Cancelled.is_cancelled());
        let unknown = EndOfTurn::Other("provider_end".into());
        assert_eq!(unknown.display_label(), "provider_end");
        assert!(!unknown.is_cancelled());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModeSelection {
    pub session: SessionId,
    pub mode: super::ids::ModeId,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigurationSelection {
    pub session: SessionId,
    pub option: super::ids::ConfigId,
    pub value: crate::session_config::ConfigValue,
}
