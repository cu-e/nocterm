//! Pure saved-chat formats: byte-stable v1 and explicitly scoped v2 envelopes.
//! V2 starts with a canonical catalog header bounded to 16 KiB; full hydration
//! checks its projection against the payload. Storage supplies scope/path authority.
//!
//! A chat keeps what the panel showed — the user's messages, the agent's
//! replies and tool calls — and the agent session it belonged to, so it can be
//! reopened after a restart. The terminal context sent with each prompt is
//! never part of the transcript, so it is never saved.
use crate::model::content::Content;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::thread::Entry;

mod budget;
mod catalog;
mod codec;
mod error;
pub use codec::{decode, encode, encode_shared, prepare, structural_bytes};
pub use error::HistoryCodecError;
mod decode;
mod loading;
mod preflight;
pub use catalog::{ChatSummary, FileStamp, decode_summary};
pub use loading::{HistoryGate, HistoryPermit};
mod catalog_header;
mod header;
mod incoming;
mod prefix;
mod size;
mod version;
mod wire;
pub use budget::{HistoryBudget, PreparedBudget};
pub use catalog_header::CatalogHeader;
pub use header::SavedHeader;
pub use nocterm_project_model::ContextId;
pub use prefix::{MAX_HEADER_BYTES, decode_catalog_header};
pub use version::TranscriptVersion;
const VERSION: u32 = 2;
const LEGACY_VERSION: u32 = 1;
/// Images larger than this are left out of saved chats.
const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
const OMITTED_IMAGE_TEXT: &str = "[Image not kept in history]";
/// Only the most recent entries of a very long chat are kept.
const MAX_SAVED_ENTRIES: usize = 2_000;
/// Larger files are not read back.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// At most this many chats are restored, newest first.
pub const MAX_RESTORED: usize = 200;
/// Maximum resident structural payload for a restored document or prepared catalog.
pub const MAX_RESIDENT_BYTES: usize = preflight::LIMIT;

/// Context references that survive application restarts. Local terminal ids do not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SavedAttachment {
    Connection(String),
    Group(String),
    /// Local shells have no stable server profile to reconnect.
    LocalTerminal(String),
}

/// A prompt waiting its turn. Images remain complete until it is dispatched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPrompt {
    pub id: u64,
    pub text: String,
    #[serde(default, deserialize_with = "decode::images")]
    pub images: Vec<Content>,
    #[serde(default, deserialize_with = "decode::attachments")]
    pub attachments: Vec<SavedAttachment>,
}

/// One saved chat. Its identity and catalog metadata share one header.
#[derive(Clone, Debug)]
pub struct SavedChat {
    pub version: u32,
    pub header: SavedHeader,
    /// The agent's session, for resuming it.
    pub session_id: Option<String>,
    /// The provider's local resume directory, not project root authority.
    pub workdir: Option<PathBuf>,
    pub model: Option<String>,
    pub config: crate::session_config::Choices,
    pub mode: Option<String>,
    pub pending_history: bool,
    /// Unsent composer text, preserved literally outside the transcript.
    pub draft: Option<String>,
    pub entries: Vec<Entry>,
    pub queue: Vec<SavedPrompt>,
    pub attachments: Vec<SavedAttachment>,
    /// When each entry began, aligned with `entries`.
    pub times: Vec<u64>,
    prepared: Option<PreparedBudget>,
    archived_draft: bool,
}
impl Serialize for SavedChat {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        wire::Wire::complete(self).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for SavedChat {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        incoming::deserialize(deserializer)
    }
}

impl SavedChat {
    /// An empty framing value for an already allocated identity and supplied timestamp.
    pub fn for_identity(agent_id: String, id: String, updated: u64) -> Self {
        Self::for_scope(
            agent_id,
            id,
            updated,
            nocterm_project_model::ContextId::General,
        )
    }
    /// Frame a newly allocated chat with an explicit durable owner.
    pub fn for_scope(
        agent_id: String,
        id: String,
        updated: u64,
        scope: nocterm_project_model::ContextId,
    ) -> Self {
        Self {
            version: VERSION,
            header: SavedHeader::new(agent_id, id, updated, scope),
            session_id: None,
            workdir: None,
            model: None,
            config: Default::default(),
            mode: None,
            pending_history: false,
            draft: None,
            entries: Vec::new(),
            queue: Vec::new(),
            attachments: Vec::new(),
            times: Vec::new(),
            prepared: None,
            archived_draft: false,
        }
    }

    /// A draft edit made before the archived transcript has been hydrated.
    pub fn archive_draft(id: String, agent_id: String, text: String, updated: u64) -> Self {
        Self::archive_draft_in_scope(
            id,
            agent_id,
            text,
            updated,
            nocterm_project_model::ContextId::General,
        )
    }
    /// A literal draft patch carries the same durable owner as its original document.
    pub fn archive_draft_in_scope(
        id: String,
        agent_id: String,
        text: String,
        updated: u64,
        scope: nocterm_project_model::ContextId,
    ) -> Self {
        let mut chat = Self::for_scope(agent_id, id, updated, scope);
        chat.draft = (!text.is_empty()).then_some(text);
        chat.header.updated = updated;
        chat.archived_draft = true;
        chat
    }
    /// The pending archived draft fields; no transcript is fabricated from this patch.
    pub fn draft_patch(&self) -> Option<(Option<String>, u64)> {
        self.archived_draft
            .then(|| (self.draft.clone(), self.header.updated))
    }
    /// Admit identity and format ownership before exposing an in-memory record.
    pub fn validate_identity(&self) -> Result<(), HistoryCodecError> {
        codec::validate_id(&self.header.id)?;
        header::resolve_scope(self.version, Some(self.header.scope))?;
        Ok(())
    }
    /// Reject a prompt before accepting it if it could not be read back.
    pub fn validate_size(&self) -> Result<(), HistoryCodecError> {
        self.validate_collections()?;
        let queue = (!self.queue.is_empty()).then_some(&self.queue);
        check_memory(
            size::structural_size(&wire::Wire::new(self, queue))
                .map_err(HistoryCodecError::from)?,
        )?;
        let queue = (!self.queue.is_empty()).then_some(&self.queue);
        check_size(
            size::encoded_len(&wire::Wire::new(self, queue)).map_err(HistoryCodecError::from)?,
        )
    }
    fn validate_collections(&self) -> Result<(), HistoryCodecError> {
        if self.queue.len() > 1024
            || self.attachments.len() > 1024
            || self
                .queue
                .iter()
                .any(|prompt| prompt.images.len() > 128 || prompt.attachments.len() > 1024)
        {
            return Err(HistoryCodecError::CollectionLimit);
        }
        Ok(())
    }
    pub fn resident_bytes(&self) -> usize {
        self.prepared
            .as_ref()
            .map_or(0, |prepared| prepared.resident_bytes)
    }
    pub fn prepared_budget(&self) -> Option<&PreparedBudget> {
        self.prepared.as_ref()
    }
    /// First row included by the version-one bounded transcript projection.
    pub fn saved_entry_start(entries_len: usize) -> usize {
        entries_len.saturating_sub(MAX_SAVED_ENTRIES)
    }
    pub fn bounded_times(entries_len: usize, times: &[u64]) -> Vec<u64> {
        times
            .iter()
            .skip(entries_len.saturating_sub(MAX_SAVED_ENTRIES))
            .copied()
            .collect()
    }
    pub fn bounded_entries(entries: &[Entry]) -> Vec<Entry> {
        wire::clone_entries(entries)
    }
}
fn check_memory(bytes: usize) -> Result<(), HistoryCodecError> {
    if bytes > preflight::LIMIT {
        Err(HistoryCodecError::StructuralLimit)
    } else {
        Ok(())
    }
}
pub fn prompt_memory(prompt: &SavedPrompt) -> Result<usize, HistoryCodecError> {
    size::structural_size(prompt).map_err(HistoryCodecError::from)
}
fn check_size(bytes: usize) -> Result<(), HistoryCodecError> {
    if bytes as u64 > MAX_FILE_BYTES {
        Err(HistoryCodecError::EncodedLimit)
    } else {
        Ok(())
    }
}
/// Runtime snapshot shares immutable pending payloads; its transcript is already bounded.
pub struct SharedChat {
    pub metadata: SavedChat,
    pub queue: Vec<std::sync::Arc<SavedPrompt>>,
}
impl SharedChat {
    /// Conservative structural ownership cost, including full retained media and
    /// shared queue records. No encoded transcript buffer is allocated.
    pub fn retained_bytes(&self) -> Result<usize, HistoryCodecError> {
        let metadata = size::structural_size(&wire::Wire::complete(&self.metadata))?;
        self.queue.iter().try_fold(metadata, |bytes, prompt| {
            Ok(bytes.saturating_add(prompt_memory(prompt)?))
        })
    }
}
impl std::ops::Deref for SharedChat {
    type Target = SavedChat;
    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}
pub fn content_size(content: &Content) -> Result<usize, HistoryCodecError> {
    size::encoded_len(content).map_err(HistoryCodecError::from)
}
pub fn prompt_size(prompt: &SavedPrompt) -> Result<usize, HistoryCodecError> {
    size::encoded_len(prompt).map_err(HistoryCodecError::from)
}

#[cfg(test)]
mod draft_tests;
#[cfg(test)]
mod scope_tests;
#[cfg(test)]
mod version_one_tests;
