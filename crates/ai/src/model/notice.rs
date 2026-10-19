//! Owned live facts; provider schemas and wire spelling remain in adapters.
use super::{
    content::Content,
    controls::{Change, ConfigOption},
    ids::{ModeId, SessionId},
    tools::{ToolPatch, ToolRecord},
    usage::ContextUsage,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq)]
pub struct SessionNotice {
    pub session: SessionId,
    pub update: SessionUpdate,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SessionUpdate {
    UserChunk(Content),
    AgentChunk(Content),
    ThoughtChunk(Content),
    ToolStarted(ToolRecord),
    ToolChanged(ToolPatch),
    Plan(Vec<PlanStep>),
    Commands(Vec<AdvertisedCommand>),
    Mode(ModeId),
    Configuration(Vec<ConfigOption>),
    Info(Change<String>),
    Usage(UsageNotice),
}
#[derive(Clone, Debug, PartialEq)]
pub struct UsageNotice {
    pub context: ContextUsage,
    pub rate_limits: Vec<crate::usage::RateLimit>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub content: String,
    pub priority: PlanPriority,
    pub status: PlanStatus,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPriority {
    High,
    Medium,
    Low,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    Running,
    Complete,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvertisedCommand {
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
}
impl AdvertisedCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_hint: None,
        }
    }
}
