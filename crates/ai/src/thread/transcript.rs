use super::Entry;
use crate::model::{
    content::Content,
    tools::{ClientDisplay, ToolPatch, ToolStatus},
};
#[derive(Clone, Debug, Default)]
pub(super) struct Transcript {
    entries: Vec<Entry>,
    times: Vec<u64>,
}
impl Transcript {
    pub(super) fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub(super) fn times(&self) -> &[u64] {
        &self.times
    }
    pub(super) fn restore(&mut self, entries: Vec<Entry>, times: Vec<u64>) {
        // Restoration preserves original optional/misaligned time facts. The
        // existing push transition aligns them only when a real row is added.
        self.entries = entries;
        self.times = times;
    }
    pub(super) fn push(&mut self, entry: Entry, now: u64) {
        self.times.resize(self.entries.len(), 0);
        self.entries.push(entry);
        self.times.push(now);
    }
    pub(super) fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
        self.times.truncate(len);
    }
    pub(super) fn user_chunk(&mut self, content: Content, now: u64) {
        if let Some(Entry::User(parts)) = self.entries.last_mut() {
            parts.push(content);
        } else {
            self.push(Entry::User(vec![content]), now);
        }
    }
    pub(super) fn chunk(&mut self, content: Content, thought: bool, now: u64) {
        if let Content::Text(text) = content {
            match (self.entries.last_mut(), thought) {
                (Some(Entry::Agent(value)), false) | (Some(Entry::Thought(value)), true) => {
                    value.push_str(&text.text)
                }
                _ => self.push(
                    if thought {
                        Entry::Thought(text.text)
                    } else {
                        Entry::Agent(text.text)
                    },
                    now,
                ),
            }
        } else {
            self.push(Entry::Content(content), now);
        }
    }
    pub(super) fn patch(&mut self, patch: ToolPatch) {
        if let Some(Entry::Tool(call)) = self
            .entries
            .iter_mut()
            .find(|entry| matches!(entry, Entry::Tool(call) if call.id == patch.id))
        {
            call.apply(patch);
        }
    }
    pub(super) fn patch_display(&mut self, row: usize, display: ClientDisplay) -> bool {
        let Some(Entry::Tool(call)) = self.entries.get_mut(row) else {
            return false;
        };
        if call.client_display.as_deref() == Some(&display) {
            return false;
        }
        call.extensions = call
            .extensions
            .take()
            .map(|values| values.without_client_display());
        call.client_display = Some(crate::model::shared_display::SharedDisplay::from_untracked(display));
        true
    }
    pub(super) fn finish_pending_tools(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        for (row, entry) in self.entries.iter_mut().enumerate() {
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
