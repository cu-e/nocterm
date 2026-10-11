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
}
