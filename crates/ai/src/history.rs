//! Saved chats: one JSON file per chat in a private directory.
//!
//! A chat keeps what the panel showed — the user's messages, the agent's
//! replies and tool calls — and the agent session it belonged to, so it can be
//! reopened after a restart. The terminal context sent with each prompt is
//! never part of the transcript, so it is never saved.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{acp, thread::Entry};

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
mod size;
mod wire;
pub use budget::{HistoryBudget, PreparedBudget};
const VERSION: u32 = 1;
/// Images larger than this are left out of saved chats.
const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
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
    pub images: Vec<acp::ContentBlock>,
    #[serde(default, deserialize_with = "decode::attachments")]
    pub attachments: Vec<SavedAttachment>,
}

/// One saved chat.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedChat {
    pub version: u32,
    /// Names the file; a UUID.
    pub id: String,
    /// The registry id of the agent the chat was held with.
    pub agent_id: String,
    /// The title the agent gave the chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The name the user gave the chat, over the agent's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Pinned chats are listed first and always restored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// The agent's session, for resuming it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The directory the session was opened in; resuming needs the same one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<PathBuf>,
    /// The model of the last prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The configuration the chat last had (model, effort, mode, …) by
    /// option id; given to every session the chat opens.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub config: crate::session_config::Choices,
    /// The mode the chat last had, for agents that report modes apart from
    /// their configuration options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Saved transcript still needs delivery to the current agent session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending_history: bool,
    /// Unsent composer text, preserved literally and kept out of the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// Seconds since the Unix epoch.
    pub updated: u64,
    #[serde(deserialize_with = "decode::entries")]
    pub entries: Vec<Entry>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decode::prompts"
    )]
    pub queue: Vec<SavedPrompt>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decode::attachments"
    )]
    pub attachments: Vec<SavedAttachment>,
    /// When each entry began, aligned with `entries`.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "decode::times"
    )]
    pub times: Vec<u64>,
    #[serde(skip)]
    prepared: Option<PreparedBudget>,
    #[serde(skip)]
    archived_draft: bool,
}

impl SavedChat {
    /// An empty framing value for an already allocated identity and supplied timestamp.
    pub fn for_identity(agent_id: String, id: String, updated: u64) -> Self {
        Self {
            version: VERSION,
            id,
            agent_id,
            title: None,
            name: None,
            pinned: false,
            session_id: None,
            workdir: None,
            model: None,
            config: Default::default(),
            mode: None,
            pending_history: false,
            draft: None,
            updated,
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
        let mut chat = Self::for_identity(agent_id, id, updated);
        chat.draft = (!text.is_empty()).then_some(text);
        chat.updated = updated;
        chat.archived_draft = true;
        chat
    }
    /// The pending archived draft fields; no transcript is fabricated from this patch.
    pub fn draft_patch(&self) -> Option<(Option<String>, u64)> {
        self.archived_draft
            .then(|| (self.draft.clone(), self.updated))
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
impl std::ops::Deref for SharedChat {
    type Target = SavedChat;
    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}
pub fn content_size(content: &acp::ContentBlock) -> Result<usize, HistoryCodecError> {
    size::encoded_len(content).map_err(HistoryCodecError::from)
}
pub fn prompt_size(prompt: &SavedPrompt) -> Result<usize, HistoryCodecError> {
    size::encoded_len(prompt).map_err(HistoryCodecError::from)
}

#[cfg(test)]
mod draft_tests;
#[cfg(test)]
mod version_one_tests;
