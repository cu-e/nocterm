//! Borrowed, bounded persistence view. JSON and byte budgeting use the same serializer.
use super::catalog_header::{CatalogHeader, Preview};
use super::{MAX_SAVED_ENTRIES, MAX_SAVED_IMAGE_BYTES, SavedChat, SavedPrompt};
use crate::model::content::Content;
use crate::model::tools::ToolRecord;
use crate::thread::Entry;
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::sync::Arc;

pub(super) struct Wire<'a, Q: Serialize + ?Sized> {
    version: u32,
    header: &'a super::SavedHeader,
    session_id: &'a Option<String>,
    workdir: &'a Option<std::path::PathBuf>,
    model: &'a Option<String>,
    config: &'a crate::session_config::Choices,
    mode: &'a Option<String>,
    pending_history: bool,
    draft: &'a Option<String>,
    entries: Entries<'a>,
    queue: Option<&'a Q>,
    attachments: &'a [super::SavedAttachment],
    times: &'a [u64],
}
impl<'a, Q: Serialize + ?Sized> Wire<'a, Q> {
    pub(super) fn new(chat: &'a SavedChat, queue: Option<&'a Q>) -> Self {
        let skip = chat.entries.len().saturating_sub(MAX_SAVED_ENTRIES);
        Self::from_entries(
            chat,
            queue,
            Entries {
                rows: &chat.entries[skip..],
                bounded: true,
                preview: Preview::persisted(&chat.entries[skip..]),
            },
            chat.times.get(skip..).unwrap_or_default(),
        )
    }
    pub(super) fn budget(chat: &'a SavedChat, queue: Option<&'a Q>, preview: Preview) -> Self {
        Self::from_entries(
            chat,
            queue,
            Entries {
                rows: &chat.entries,
                bounded: true,
                preview,
            },
            &chat.times,
        )
    }
    fn from_entries(
        chat: &'a SavedChat,
        queue: Option<&'a Q>,
        entries: Entries<'a>,
        times: &'a [u64],
    ) -> Self {
        Self {
            version: chat.version,
            header: &chat.header,
            session_id: &chat.session_id,
            workdir: &chat.workdir,
            model: &chat.model,
            config: &chat.config,
            mode: &chat.mode,
            pending_history: chat.pending_history,
            draft: &chat.draft,
            entries,
            queue,
            attachments: &chat.attachments,
            times,
        }
    }
}
impl<'a> Wire<'a, Vec<SavedPrompt>> {
    pub(super) fn complete(chat: &'a SavedChat) -> Self {
        Self::from_entries(
            chat,
            (!chat.queue.is_empty()).then_some(&chat.queue),
            Entries {
                rows: &chat.entries,
                bounded: false,
                preview: Preview::new(&chat.entries),
            },
            &chat.times,
        )
    }
}
impl<Q: Serialize + ?Sized> Serialize for Wire<'_, Q> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        super::header::resolve_scope(self.version, Some(self.header.scope))
            .map_err(serde::ser::Error::custom)?;
        if self.version == super::LEGACY_VERSION {
            return Flat(self).serialize(serializer);
        }
        let catalog = CatalogHeader::from_parts(
            self.header,
            self.model,
            &self.entries.preview,
            self.draft.as_deref(),
            self.queue.is_some(),
        );
        if super::size::encoded_len(&catalog).map_err(serde::ser::Error::custom)?
            + super::prefix::START.len()
            + super::prefix::PAYLOAD.len()
            > super::MAX_HEADER_BYTES
        {
            return Err(serde::ser::Error::custom(
                super::HistoryCodecError::HeaderLimit,
            ));
        }
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("version", &self.version)?;
        map.serialize_entry("catalog", &catalog)?;
        map.serialize_entry("chat", &Flat(self))?;
        map.end()
    }
}
struct Flat<'a, 'b, Q: Serialize + ?Sized>(&'b Wire<'a, Q>);
impl<Q: Serialize + ?Sized> Serialize for Flat<'_, '_, Q> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("version", &wire.version)?;
        wire.header.serialize_prefix(&mut map)?;
        if wire.version != super::LEGACY_VERSION {
            map.serialize_entry("scope", &wire.header.scope)?;
        }
        optional(&mut map, "session_id", wire.session_id)?;
        optional(&mut map, "workdir", wire.workdir)?;
        optional(&mut map, "model", wire.model)?;
        if !wire.config.is_empty() {
            map.serialize_entry("config", wire.config)?;
        }
        optional(&mut map, "mode", wire.mode)?;
        if wire.pending_history {
            map.serialize_entry("pending_history", &wire.pending_history)?;
        }
        optional(&mut map, "draft", wire.draft)?;
        map.serialize_entry("updated", &wire.header.updated)?;
        map.serialize_entry("entries", &wire.entries)?;
        if let Some(queue) = wire.queue {
            map.serialize_entry("queue", queue)?;
        }
        if !wire.attachments.is_empty() {
            map.serialize_entry("attachments", wire.attachments)?;
        }
        if !wire.times.is_empty() {
            map.serialize_entry("times", wire.times)?;
        }
        map.end()
    }
}
fn optional<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}
pub(super) struct Entries<'a> {
    rows: &'a [Entry],
    bounded: bool,
    preview: Preview,
}
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        for entry in self.rows {
            if self.bounded {
                seq.serialize_element(&bounded(entry))?;
            } else {
                seq.serialize_element(entry)?;
            }
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
    Tool(&'a ToolRecord),
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
struct Blocks<'a>(&'a [Content]);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for block in self.0 {
            seq.serialize_element(&Block(block))?;
        }
        seq.end()
    }
}
struct Block<'a>(&'a Content);
impl Serialize for Block<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Content::Image(image) if image.data.len() > MAX_SAVED_IMAGE_BYTES => {
                Content::text(super::OMITTED_IMAGE_TEXT).serialize(serializer)
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
pub(super) fn entry_memory(entry: &Entry) -> Result<usize, serde_json::Error> {
    super::size::structural_size(&bounded(entry))
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
fn clone_block(block: &Content) -> Content {
    match block {
        Content::Image(image) if image.data.len() > MAX_SAVED_IMAGE_BYTES => {
            Content::text(super::OMITTED_IMAGE_TEXT)
        }
        block => block.clone(),
    }
}
