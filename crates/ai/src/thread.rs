use crate::model::{
    content::Content,
    controls::Change,
    plan::Plan,
    session::SessionUpdate,
    tools::ToolRecord,
    usage::{ContextUsage, TokenTotals},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Entry {
    User(Vec<Content>),
    Agent(String),
    Thought(String),
    Content(Content),
    Tool(ToolRecord),
}
#[derive(Clone, Debug, Default)]
pub struct ThreadState {
    pub entries: Vec<Entry>,
    /// When each entry began, in seconds since the Unix epoch; 0 where it
    /// is not known (chats saved before times were kept).
    pub times: Vec<u64>,
    pub plan: Option<Plan>,
    pub usage: Option<ContextUsage>,
    /// The session's token totals as of the last finished turn.
    pub tokens: Option<TokenTotals>,
    pub title: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadChange {
    Transcript,
    Controls,
    Metadata,
    Ignored,
}
impl ThreadState {
    pub fn push_user(&mut self, content: Vec<Content>, now: u64) {
        self.push(Entry::User(content), now);
    }
    fn push(&mut self, entry: Entry, now: u64) {
        self.times.resize(self.entries.len(), 0);
        self.entries.push(entry);
        self.times.push(now);
    }
    /// When entry `index` began, if known.
    pub fn time(&self, index: usize) -> Option<u64> {
        self.times.get(index).copied().filter(|time| *time > 0)
    }
    /// Keeps the first `len` entries.
    pub fn truncate(&mut self, len: usize) {
        self.entries.truncate(len);
        self.times.truncate(len);
    }
    pub fn apply(&mut self, update: SessionUpdate, now: u64) -> ThreadChange {
        match update {
            SessionUpdate::AgentMessageChunk(content) => {
                self.chunk(content, false, now);
                ThreadChange::Transcript
            }
            SessionUpdate::AgentThoughtChunk(content) => {
                self.chunk(content, true, now);
                ThreadChange::Transcript
            }
            SessionUpdate::UserMessageChunk(content) => {
                if let Some(Entry::User(parts)) = self.entries.last_mut() {
                    parts.push(content);
                } else {
                    self.push_user(vec![content], now);
                }
                ThreadChange::Transcript
            }
            SessionUpdate::ToolCall(call) => {
                self.push(Entry::Tool(*call), now);
                ThreadChange::Transcript
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let update = *update;
                if let Some(Entry::Tool(call)) = self
                    .entries
                    .iter_mut()
                    .find(|e| matches!(e, Entry::Tool(t) if t.id == update.id))
                {
                    call.apply(update.changes);
                }
                ThreadChange::Transcript
            }
            SessionUpdate::Plan(plan) => {
                self.plan = Some(plan);
                ThreadChange::Transcript
            }
            SessionUpdate::SessionInfoUpdate(update) => {
                match update.title {
                    Change::Set(value) => self.title = Some(value),
                    Change::Clear => self.title = None,
                    Change::Keep => {}
                }
                ThreadChange::Metadata
            }
            SessionUpdate::UsageUpdate(update) => {
                self.usage = Some(update);
                ThreadChange::Controls
            }
            _ => ThreadChange::Ignored,
        }
    }
    fn chunk(&mut self, content: Content, thought: bool, now: u64) {
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
    /// Whether the chat's title or any message mentions `query`, ignoring
    /// case. `query` must be lowercase.
    pub fn mentions(&self, query: &str) -> bool {
        let found = |text: &str| text.to_lowercase().contains(query);
        self.title.as_deref().is_some_and(found)
            || self.entries.iter().any(|entry| match entry {
                Entry::User(blocks) => blocks
                    .iter()
                    .any(|block| matches!(block, Content::Text(text) if found(&text.text))),
                Entry::Agent(text) => found(text),
                _ => false,
            })
    }
}
