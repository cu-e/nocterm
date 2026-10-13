//! Presentation state reduces legacy updates; only actual saved-row changes are semantic.
use nocterm_ai::{
    acp,
    thread::{Entry, ThreadChange, ThreadState},
};

#[derive(Clone, Default)]
pub(super) struct Transcript {
    state: ThreadState,
}
pub(crate) struct ToolMetadataPatch {
    row: usize,
    value: serde_json::Value,
}
impl ToolMetadataPatch {
    pub(crate) fn new(row: usize, value: serde_json::Value) -> Self {
        Self { row, value }
    }
    pub(crate) fn row(&self) -> usize {
        self.row
    }
}
impl Transcript {
    pub(super) fn restored(entries: Vec<Entry>, times: Vec<u64>, title: Option<String>) -> Self {
        Self {
            state: ThreadState {
                entries,
                times,
                title,
                ..Default::default()
            },
        }
    }
    pub(super) fn state(&self) -> &ThreadState {
        &self.state
    }
    pub(super) fn apply(&mut self, update: acp::SessionUpdate) -> (ThreadChange, bool) {
        let semantic = self.changes_saved(&update);
        (self.state.apply(update), semantic)
    }
    fn changes_saved(&self, update: &acp::SessionUpdate) -> bool {
        match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => !matches!((&chunk.content, self.state.entries.last()), (acp::ContentBlock::Text(text), Some(Entry::Agent(_))) if text.text.is_empty()),
            acp::SessionUpdate::AgentThoughtChunk(chunk) => !matches!((&chunk.content, self.state.entries.last()), (acp::ContentBlock::Text(text), Some(Entry::Thought(_))) if text.text.is_empty()),
            acp::SessionUpdate::UserMessageChunk(_) | acp::SessionUpdate::ToolCall(_) => true,
            acp::SessionUpdate::ToolCallUpdate(update) => self.state.entries.iter().enumerate().find(|(_, entry)| matches!(entry, Entry::Tool(call) if call.tool_call_id == update.tool_call_id)).is_some_and(|(row, entry)| {
                row >= nocterm_ai::history::SavedChat::saved_entry_start(self.state.entries.len()) && match entry {
                    Entry::Tool(call) => changed_tool(call, &update.fields),
                    _ => false,
                }
            }),
            acp::SessionUpdate::SessionInfoUpdate(update) => update.title.as_opt_ref().is_some_and(|title| title != self.state.title.as_ref()),
            _ => false,
        }
    }
    pub(super) fn append_user(&mut self, content: Vec<acp::ContentBlock>) {
        self.state.push_user(content);
    }
    pub(super) fn tokens(&mut self, usage: Option<acp::Usage>) {
        self.state.tokens = usage;
    }
    pub(super) fn patch_tool(&mut self, patch: ToolMetadataPatch) -> bool {
        let Some(Entry::Tool(call)) = self.state.entries.get_mut(patch.row) else {
            return false;
        };
        let key = nocterm_ai::tool_display::META_KEY;
        if call.meta.as_ref().and_then(|meta| meta.get(key)) == Some(&patch.value) {
            return false;
        }
        call.meta
            .get_or_insert_with(Default::default)
            .insert(key.into(), patch.value);
        true
    }
    pub(super) fn finish_pending_tools(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        for (row, entry) in self.state.entries.iter_mut().enumerate() {
            if let Entry::Tool(call) = entry
                && matches!(
                    call.status,
                    acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress
                )
            {
                call.status = acp::ToolCallStatus::Failed;
                changed.push(row);
            }
        }
        changed
    }
}
fn changed_tool(call: &acp::ToolCall, fields: &acp::ToolCallUpdateFields) -> bool {
    fields
        .kind
        .as_ref()
        .is_some_and(|value| value != &call.kind)
        || fields
            .status
            .as_ref()
            .is_some_and(|value| value != &call.status)
        || fields
            .title
            .as_ref()
            .is_some_and(|value| value != &call.title)
        || fields
            .name
            .as_ref()
            .is_some_and(|value| Some(value) != call.name.as_ref())
        || fields
            .content
            .as_ref()
            .is_some_and(|value| value != &call.content)
        || fields
            .locations
            .as_ref()
            .is_some_and(|value| value != &call.locations)
        || fields
            .raw_input
            .as_ref()
            .is_some_and(|value| Some(value) != call.raw_input.as_ref())
        || fields
            .raw_output
            .as_ref()
            .is_some_and(|value| Some(value) != call.raw_output.as_ref())
}
