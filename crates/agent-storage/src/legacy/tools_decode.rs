//! Version-one tool fields retain original lenient rules and consuming content.
use super::content_decode::Block;
use agent_client_protocol_schema::v1 as wire;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, VecSkipError, serde_as};

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Call {
    pub(super) tool_call_id: String,
    pub(super) title: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "kind_default")]
    pub(super) kind: wire::ToolKind,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "status_default")]
    pub(super) status: wire::ToolCallStatus,
    #[serde_as(deserialize_as = "DefaultOnError<VecSkipError<_>>")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) content: Vec<Output>,
    #[serde_as(deserialize_as = "DefaultOnError<VecSkipError<_>>")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) locations: Vec<wire::ToolCallLocation>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) raw_input: Option<serde_json::Value>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) raw_output: Option<serde_json::Value>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub(super) meta: Option<wire::Meta>,
}
fn kind_default(value: &wire::ToolKind) -> bool {
    matches!(value, wire::ToolKind::Other)
}
fn status_default(value: &wire::ToolCallStatus) -> bool {
    matches!(value, wire::ToolCallStatus::Pending)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Output {
    Content(Box<ToolContent>),
    Diff(Box<wire::Diff>),
    Terminal(Box<Terminal>),
}
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ToolContent {
    pub(super) content: Block,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub(super) meta: Option<wire::Meta>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Terminal {
    pub(super) terminal_id: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub(super) meta: Option<wire::Meta>,
}
