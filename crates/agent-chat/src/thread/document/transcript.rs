//! Presentation state reduces legacy updates; only actual saved-row changes are semantic.
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::Change;
use nocterm_ai::model::session::SessionUpdate;
use nocterm_ai::model::tools::{ToolChanges, ToolRecord, ToolStatus};
use nocterm_ai::model::usage::TokenTotals;
use nocterm_ai::thread::{Entry, ThreadChange, ThreadState};

#[derive(Clone)]
pub(super) struct Transcript {
    state: ThreadState,
    live_bytes: usize,
}
impl Default for Transcript {
    fn default() -> Self {
        Self::restored(Vec::new(), Vec::new(), None)
    }
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
        let state = ThreadState {
            entries,
            times,
            title,
            ..Default::default()
        };
        let live_bytes = nocterm_ai::thread::live::resident_bytes(&state).unwrap_or(usize::MAX);
        Self { state, live_bytes }
    }
    pub(super) fn state(&self) -> &ThreadState {
        &self.state
    }
    pub(super) fn apply(
        &mut self,
        update: SessionUpdate,
        now: u64,
    ) -> Result<(ThreadChange, bool), nocterm_ai::history::HistoryCodecError> {
        let next = nocterm_ai::thread::live::admit_update(&self.state, self.live_bytes, &update)?;
        let semantic = self.changes_saved(&update);
        let change = self.state.apply(update, now);
        self.live_bytes = next;
        Ok((change, semantic))
    }
    fn changes_saved(&self, update: &SessionUpdate) -> bool {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                !matches!((chunk, self.state.entries.last()), (Content::Text(text), Some(Entry::Agent(_))) if text.text.is_empty())
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                !matches!((chunk, self.state.entries.last()), (Content::Text(text), Some(Entry::Thought(_))) if text.text.is_empty())
            }
            SessionUpdate::UserMessageChunk(_) | SessionUpdate::ToolCall(_) => true,
            SessionUpdate::ToolCallUpdate(update) => self
                .state
                .entries
                .iter()
                .enumerate()
                .find(|(_, entry)| matches!(entry, Entry::Tool(call) if call.id == update.id))
                .is_some_and(|(row, entry)| {
                    row >= nocterm_ai::history::SavedChat::saved_entry_start(
                        self.state.entries.len(),
                    ) && match entry {
                        Entry::Tool(call) => changed_tool(call, &update.changes),
                        _ => false,
                    }
                }),
            SessionUpdate::SessionInfoUpdate(update) => {
                matches!(&update.title, Change::Set(title) if Some(title) != self.state.title.as_ref())
            }
            _ => false,
        }
    }
    /// A transcript of the same chat made of `updates` alone, oldest first.
    pub(super) fn replaced(
        &self,
        updates: Vec<SessionUpdate>,
        now: u64,
    ) -> Result<Self, nocterm_ai::history::HistoryCodecError> {
        let mut next = Self::restored(Vec::new(), Vec::new(), self.state.title.clone());
        for update in updates {
            next.apply(update, now)?;
        }
        Ok(next)
    }
    pub(super) fn append_user(
        &mut self,
        content: Vec<Content>,
        now: u64,
    ) -> Result<(), nocterm_ai::history::HistoryCodecError> {
        self.live_bytes =
            nocterm_ai::thread::live::admit_user(&self.state, self.live_bytes, &content)?;
        self.state.push_user(content, now);
        Ok(())
    }
    pub(super) fn resident_bytes(&self) -> usize {
        self.live_bytes
    }
    pub(super) fn tokens(&mut self, usage: Option<TokenTotals>) {
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
        let new_key = call.meta.as_ref().and_then(|meta| meta.get(key)).is_none();
        let removed = call
            .meta
            .as_ref()
            .and_then(|meta| meta.get(key))
            .map_or(Ok(0), nocterm_ai::history::retained_value_bytes)
            .unwrap_or(0);
        let added = nocterm_ai::history::retained_value_bytes(&patch.value).unwrap_or(usize::MAX);
        let next = self
            .live_bytes
            .saturating_sub(removed)
            .saturating_add(added)
            .saturating_add(if new_key { 2048 } else { 0 });
        if next > nocterm_ai::history::MAX_RESIDENT_BYTES {
            return false;
        }
        self.live_bytes = next;
        call.meta
            .get_or_insert_with(Default::default)
            .insert(key.into(), patch.value);
        true
    }
    pub(super) fn finish_pending_tools(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        for (row, entry) in self.state.entries.iter_mut().enumerate() {
            if let Entry::Tool(call) = entry
                && matches!(call.status, ToolStatus::Pending | ToolStatus::Running)
            {
                call.status = ToolStatus::Failed;
                changed.push(row);
            }
        }
        changed
    }
}
fn changed_tool(call: &ToolRecord, fields: &ToolChanges) -> bool {
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
