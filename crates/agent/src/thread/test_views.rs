//! Plain copies of the retained rows and identities, for behavior assertions.
use super::AgentThread;
use nocterm_ai::{
    history::physical::{ContentSlot, RowBody},
    model::tools::ToolStatus,
};
use serde_json::Value;

/// One tool row as the transcript shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolFacts {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: ToolStatus,
    pub(crate) arguments: Option<Value>,
    pub(crate) result: Option<Value>,
}
/// One transcript row, reduced to what a test asserts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RowFacts {
    User(String),
    Agent(String),
    Thought(String),
    Content,
    Tool(ToolFacts),
}
impl AgentThread {
    pub(crate) fn row_facts(&self) -> Vec<RowFacts> {
        self.history_rows()
            .iter()
            .map(|row| match row.view() {
                RowBody::User(user) => RowFacts::User(
                    user.contents()
                        .filter_map(|content| match content {
                            ContentSlot::Text(text) => Some(text.text().to_string()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                ),
                RowBody::Agent(text) => RowFacts::Agent(text.read().to_string()),
                RowBody::Thought(text) => RowFacts::Thought(text.read().to_string()),
                RowBody::Content(_) => RowFacts::Content,
                RowBody::Tool(tool) => RowFacts::Tool(ToolFacts {
                    id: tool.id().as_str().to_owned(),
                    title: tool.title().to_string(),
                    status: tool.status(),
                    arguments: tool.arguments().map(|value| value.as_value().clone()),
                    result: tool.result().map(|value| value.as_value().clone()),
                }),
            })
            .collect()
    }
    pub(crate) fn tool_facts(&self) -> Vec<ToolFacts> {
        self.row_facts()
            .into_iter()
            .filter_map(|row| match row {
                RowFacts::Tool(tool) => Some(tool),
                _ => None,
            })
            .collect()
    }
    /// Marks the chat as relying on its history being replayed to the agent.
    pub(crate) fn mark_history_pending(&mut self) {
        let now = self.state.updated();
        self.state.set_pending_history(true, now).unwrap();
    }
    pub(crate) fn chat_id_string(&self) -> String {
        self.chat_id().map(|id| id.to_string()).unwrap_or_default()
    }
}
impl super::composer::AttachmentSelection<'_> {
    pub(crate) fn to_vec(self) -> Vec<super::Attachment> {
        self.iter().map(|value| value.to_attachment()).collect()
    }
}
impl super::tool_runtime::TurnAttachments<'_> {
    pub(crate) fn into_vec(self) -> Vec<super::Attachment> {
        self.map(|value| value.to_attachment()).collect()
    }
}
impl AgentThread {
    /// Runs `read` on the tool row at `index`, which must be a tool row.
    pub(crate) fn with_tool<R>(
        &self,
        index: usize,
        read: impl FnOnce(&nocterm_ai::history::physical::ToolSlot) -> R,
    ) -> R {
        match self.history_rows()[index].view() {
            RowBody::Tool(tool) => read(tool),
            _ => panic!("row {index} is not a tool row"),
        }
    }
}
impl AgentThread {
    /// The tool row at `index`, which must be a tool row.
    pub(crate) fn tool_slot(&self, index: usize) -> &nocterm_ai::history::physical::ToolSlot {
        match self.history_rows()[index].view() {
            RowBody::Tool(tool) => tool,
            _ => panic!("row {index} is not a tool row"),
        }
    }
}
impl AgentThread {
    /// The attachments the current turn's tools may use.
    pub(crate) fn scope_attachments(&self) -> Vec<super::Attachment> {
        self.attachment_scope().into_vec()
    }
}
