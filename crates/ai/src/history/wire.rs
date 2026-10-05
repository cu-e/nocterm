//! Borrowed, bounded persistence view. JSON and byte budgeting use the same serializer.
use super::{MAX_SAVED_ENTRIES, MAX_SAVED_IMAGE_BYTES, SavedChat, SavedPrompt};
use crate::{acp, thread::Entry};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct Wire<'a, Q: Serialize + ?Sized> {
    version: u32,
    id: &'a str,
    agent_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: &'a Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workdir: &'a Option<std::path::PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: &'a Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pending_history: bool,
    updated: u64,
    entries: Entries<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue: Option<&'a Q>,
    #[serde(skip_serializing_if = "<[super::SavedAttachment]>::is_empty")]
    attachments: &'a [super::SavedAttachment],
    #[serde(skip_serializing_if = "<[u64]>::is_empty")]
    times: &'a [u64],
}
impl<'a, Q: Serialize + ?Sized> Wire<'a, Q> {
    pub(super) fn new(chat: &'a SavedChat, queue: Option<&'a Q>) -> Self {
        let skip = chat.entries.len().saturating_sub(MAX_SAVED_ENTRIES);
        Self {
            version: chat.version,
            id: &chat.id,
            agent_id: &chat.agent_id,
            title: &chat.title,
            name: &chat.name,
            pinned: chat.pinned,
            session_id: &chat.session_id,
            workdir: &chat.workdir,
            model: &chat.model,
            pending_history: chat.pending_history,
            updated: chat.updated,
            entries: Entries(&chat.entries[skip..]),
            queue,
            attachments: &chat.attachments,
            times: chat.times.get(skip..).unwrap_or_default(),
        }
    }
}
pub(super) struct Entries<'a>(pub &'a [Entry]);
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            seq.serialize_element(&bounded(entry))?;
        }
        seq.end()
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum BorrowedEntry<'a> {
    User(Blocks<'a>),
    Agent(&'a str),
    Thought(&'a str),
    Content(Block<'a>),
    Tool(&'a acp::ToolCall),
}
fn bounded(entry: &Entry) -> BorrowedEntry<'_> {
    match entry {
        Entry::User(blocks) => BorrowedEntry::User(Blocks(blocks)),
        Entry::Agent(text) => BorrowedEntry::Agent(text),
        Entry::Thought(text) => BorrowedEntry::Thought(text),
        Entry::Content(block) => BorrowedEntry::Content(Block(block)),
        Entry::Tool(tool) => BorrowedEntry::Tool(tool),
    }
}
struct Blocks<'a>(&'a [acp::ContentBlock]);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for block in self.0 {
            seq.serialize_element(&Block(block))?;
        }
        seq.end()
    }
}
struct Block<'a>(&'a acp::ContentBlock);
impl Serialize for Block<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            acp::ContentBlock::Image(image) if image.data.len() > MAX_SAVED_IMAGE_BYTES => {
                acp::ContentBlock::Text(acp::TextContent::new("[Image not kept in history]"))
                    .serialize(serializer)
            }
            block => block.serialize(serializer),
        }
    }
}
pub(super) struct Prompts<'a>(pub &'a [Arc<SavedPrompt>]);
impl Serialize for Prompts<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for prompt in self.0 {
            seq.serialize_element(&**prompt)?;
        }
        seq.end()
    }
}
pub(super) fn entry_size(entry: &Entry) -> Result<usize, serde_json::Error> {
    super::size::encoded_len(&bounded(entry))
}
pub(super) fn clone_entries(entries: &[Entry]) -> Vec<Entry> {
    entries[entries.len().saturating_sub(MAX_SAVED_ENTRIES)..]
        .iter()
        .map(|entry| match entry {
            Entry::User(blocks) => Entry::User(blocks.iter().map(clone_block).collect()),
            Entry::Content(block) => Entry::Content(clone_block(block)),
            entry => entry.clone(),
        })
        .collect()
}
fn clone_block(block: &acp::ContentBlock) -> acp::ContentBlock {
    match block {
        acp::ContentBlock::Image(image) if image.data.len() > MAX_SAVED_IMAGE_BYTES => {
            acp::ContentBlock::Text(acp::TextContent::new("[Image not kept in history]"))
        }
        block => block.clone(),
    }
}
