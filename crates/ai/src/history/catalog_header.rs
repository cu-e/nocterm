//! Bounded authoritative discovery facts live before the transcript payload.
use super::{ContextId, FileStamp, SavedChat, SavedHeader, TranscriptVersion};
use crate::{model::content::Content, thread::Entry};
use serde::{Deserialize, Serialize};

/// The catalog part of a v2 transcript, not an independently trusted sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogHeader {
    id: String,
    agent_id: String,
    scope: ContextId,
    title: Option<String>,
    name: Option<String>,
    pinned: bool,
    updated: u64,
    model: Option<String>,
    has_content: bool,
}
impl CatalogHeader {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn scope(&self) -> ContextId {
        self.scope
    }
    pub fn summary(&self, stamp: FileStamp) -> Option<ChatSummary> {
        (self.has_content || self.name.is_some() || self.pinned).then(|| self.to_summary(stamp))
    }
    pub(super) fn to_summary(&self, stamp: FileStamp) -> ChatSummary {
        ChatSummary {
            cache_version: 3,
            transcript_version: TranscriptVersion::SCOPED,
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            scope: self.scope,
            title: self.title.clone(),
            name: self.name.clone(),
            pinned: self.pinned,
            updated: self.updated,
            model: self.model.clone(),
            has_content: self.has_content,
            bytes: stamp.bytes,
            modified_nanos: stamp.modified_nanos,
        }
    }
    pub(super) fn from_chat(chat: &SavedChat, queued: bool) -> Self {
        let start = SavedChat::saved_entry_start(chat.entries.len());
        Self::from_parts(
            &chat.header,
            &chat.model,
            &Preview::new(&chat.entries[start..]),
            chat.draft.as_deref(),
            queued || !chat.queue.is_empty(),
        )
    }
    pub(super) fn for_persistence(chat: &SavedChat, queued: bool) -> Self {
        let start = SavedChat::saved_entry_start(chat.entries.len());
        Self::from_parts(
            &chat.header,
            &chat.model,
            &Preview::persisted(&chat.entries[start..]),
            chat.draft.as_deref(),
            queued || !chat.queue.is_empty(),
        )
    }
    pub(super) fn from_parts(
        saved: &SavedHeader,
        model: &Option<String>,
        preview: &Preview,
        draft: Option<&str>,
        queued: bool,
    ) -> Self {
        Self {
            id: saved.id.clone(),
            agent_id: bounded(&saved.agent_id),
            scope: saved.scope,
            title: saved
                .title
                .as_deref()
                .map(bounded)
                .or_else(|| preview.title.clone()),
            name: saved.name.as_deref().map(bounded),
            pinned: saved.pinned,
            updated: saved.updated,
            model: model.as_deref().map(bounded),
            has_content: preview.has_content
                || queued
                || draft.is_some_and(|text| !text.is_empty()),
        }
    }
    pub(super) fn validate(&self, id: &str) -> Result<(), super::HistoryCodecError> {
        if self.id != id {
            return Err(super::HistoryCodecError::IdentityMismatch);
        }
        validate_metadata(id, &self.agent_id, [&self.title, &self.name, &self.model])
    }
}
fn validate_metadata(
    id: &str,
    agent: &str,
    display: [&Option<String>; 3],
) -> Result<(), super::HistoryCodecError> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(super::HistoryCodecError::InvalidId(id.into()));
    }
    if std::iter::once(agent)
        .chain(display.into_iter().filter_map(|text| text.as_deref()))
        .any(|text| text.chars().take(513).count() > 512)
    {
        return Err(super::HistoryCodecError::HeaderLimit);
    }
    Ok(())
}
fn bounded(text: &str) -> String {
    text.chars().take(512).collect()
}

/// Small content facts cached with per-row budgets, including after dirty-row edits.
#[derive(Clone, Debug, Default)]
pub(super) struct Preview {
    title: Option<String>,
    has_content: bool,
}
impl Preview {
    pub(super) fn for_title(title: Option<String>, has_content: bool) -> Self {
        Self { title, has_content }
    }
    pub(super) fn new(entries: &[Entry]) -> Self {
        Self::projected(entries, false)
    }
    pub(super) fn persisted(entries: &[Entry]) -> Self {
        Self::projected(entries, true)
    }
    fn projected(entries: &[Entry], persisted: bool) -> Self {
        let title = entries.iter().find_map(|entry| match entry {
            Entry::User(blocks) => blocks.iter().find_map(|block| {
                let text = match block {
                    Content::Text(text) => text.text.as_str(),
                    Content::Image(image)
                        if persisted && image.data.len() > super::MAX_SAVED_IMAGE_BYTES =>
                    {
                        super::OMITTED_IMAGE_TEXT
                    }
                    _ => return None,
                };
                (!text.trim().is_empty()).then(|| text.trim().chars().take(60).collect())
            }),
            _ => None,
        });
        Self {
            title,
            has_content: !entries.is_empty(),
        }
    }
    pub(super) fn with_prompts(mut self, present: bool) -> Self {
        self.has_content |= present;
        self
    }
}

/// Rebuildable sidecar data. Only the authoritative transcript header supplies it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatSummary {
    #[serde(default)]
    cache_version: u32,
    transcript_version: TranscriptVersion,
    pub id: String,
    pub agent_id: String,
    pub scope: ContextId,
    pub title: Option<String>,
    pub name: Option<String>,
    pub pinned: bool,
    pub updated: u64,
    pub model: Option<String>,
    pub has_content: bool,
    pub bytes: u64,
    modified_nanos: u128,
}
impl ChatSummary {
    /// Check the bounded metadata contract when accepting a catalog port's DTO.
    /// This does not validate an unread transcript payload or issue authority.
    pub fn validate(&self) -> Result<(), super::HistoryCodecError> {
        self.transcript_version.require_scope(self.scope)?;
        validate_metadata(
            &self.id,
            &self.agent_id,
            [&self.title, &self.name, &self.model],
        )
    }
    pub fn transcript_version(&self) -> TranscriptVersion {
        self.transcript_version
    }
    pub(super) fn with_version(mut self, version: TranscriptVersion) -> Self {
        self.transcript_version = version;
        self
    }
    pub fn from_chat(chat: &SavedChat) -> Self {
        CatalogHeader::for_persistence(chat, false)
            .to_summary(FileStamp {
                bytes: 0,
                modified_nanos: 0,
            })
            .with_version(TranscriptVersion::observed(chat.version))
    }
    pub fn with_stamp(mut self, stamp: FileStamp, queued: bool) -> Self {
        self.bytes = stamp.bytes;
        self.modified_nanos = stamp.modified_nanos;
        self.has_content |= queued;
        self
    }
    pub fn matches_stamp(&self, id: &str, stamp: FileStamp) -> bool {
        self.cache_version == 3
            && self.id == id
            && self.bytes == stamp.bytes
            && self.modified_nanos == stamp.modified_nanos
    }
    pub fn title(&self) -> &str {
        self.name
            .as_deref()
            .or(self.title.as_deref())
            .unwrap_or("Saved chat")
    }
}
