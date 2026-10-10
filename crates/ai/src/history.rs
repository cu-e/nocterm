//! Saved chats: one JSON file per chat in a private directory.
//!
//! A chat keeps what the panel showed — the user's messages, the agent's
//! replies and tool calls — and the agent session it belonged to, so it can be
//! reopened after a restart. The terminal context sent with each prompt is
//! never part of the transcript, so it is never saved.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{acp, thread::Entry, time::now};

mod budget;
mod catalog;
mod decode;
mod loading;
mod preflight;
pub use catalog::{ChatSummary, load, load_catalog};
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
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// At most this many chats are restored, newest first.
pub const MAX_RESTORED: usize = 200;

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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// A new, empty chat with `agent_id`.
    pub fn new(agent_id: String) -> Self {
        Self {
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
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
            updated: now(),
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
        let mut chat = Self::new(agent_id);
        chat.id = id;
        chat.draft = (!text.is_empty()).then_some(text);
        chat.updated = updated;
        chat.archived_draft = true;
        chat
    }
    /// Reject a prompt before accepting it if it could not be read back.
    pub fn validate_size(&self) -> Result<(), String> {
        self.validate_collections()?;
        let queue = (!self.queue.is_empty()).then_some(&self.queue);
        check_memory(
            size::structural_size(&wire::Wire::new(self, queue))
                .map_err(|error| error.to_string())?,
        )?;
        let queue = (!self.queue.is_empty()).then_some(&self.queue);
        check_size(
            size::encoded_len(&wire::Wire::new(self, queue)).map_err(|error| error.to_string())?,
        )
    }
    fn validate_collections(&self) -> Result<(), String> {
        if self.queue.len() > 1024
            || self.attachments.len() > 1024
            || self
                .queue
                .iter()
                .any(|prompt| prompt.images.len() > 128 || prompt.attachments.len() > 1024)
        {
            return Err("This chat exceeds the saved collection limit.".into());
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
fn check_memory(bytes: usize) -> Result<(), String> {
    if bytes > preflight::LIMIT {
        Err("This chat exceeds the structural memory limit. Shorten the message or remove attachments.".into())
    } else {
        Ok(())
    }
}
pub fn prompt_memory(prompt: &SavedPrompt) -> Result<usize, String> {
    size::structural_size(prompt).map_err(|error| error.to_string())
}
fn check_size(bytes: usize) -> Result<(), String> {
    if bytes as u64 > MAX_FILE_BYTES {
        Err("The queued message exceeds this chat's storage limit. Remove images or shorten the message.".into())
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
pub fn content_size(content: &acp::ContentBlock) -> Result<usize, String> {
    size::encoded_len(content).map_err(|error| error.to_string())
}
pub fn prompt_size(prompt: &SavedPrompt) -> Result<usize, String> {
    size::encoded_len(prompt).map_err(|error| error.to_string())
}
/// Called on the background writer; one physical serialization, followed by the size check.
pub fn save_shared(dir: &Path, chat: &SharedChat) -> Result<(), String> {
    if chat.metadata.archived_draft {
        let mut original = load(dir, &chat.id)?;
        original.draft = chat.metadata.draft.clone();
        original.updated = chat.updated;
        return save(dir, &original);
    }
    if chat.queue.len() > 1024
        || chat
            .queue
            .iter()
            .any(|prompt| prompt.images.len() > 128 || prompt.attachments.len() > 1024)
    {
        return Err("This chat exceeds the saved collection limit.".into());
    }
    chat.metadata.validate_collections()?;
    let prompts = wire::Prompts(&chat.queue);
    let queue = (!chat.queue.is_empty()).then_some(&prompts);
    write(
        dir,
        &chat.metadata,
        &wire::Wire::new(&chat.metadata, queue),
        !chat.queue.is_empty(),
    )
}
fn write(dir: &Path, chat: &SavedChat, value: &impl Serialize, queued: bool) -> Result<(), String> {
    let path = file(dir, &chat.id).ok_or("invalid chat id")?;
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    check_size(text.len())?;
    preflight::check(text.as_bytes())?;
    nocterm_core::paths::ensure_private_dir(dir).map_err(|error| error.to_string())?;
    nocterm_core::persist::save_text(&path, &text).map_err(|error| error.to_string())?;
    catalog::update_cache(dir, chat, queued);
    Ok(())
}

/// Whether `id` can name a chat file: a UUID, nothing that walks paths.
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

fn file(dir: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| dir.join(format!("{id}.json")))
}

/// Writes `chat` into `dir`, creating `dir` private to the user.
pub fn save(dir: &Path, chat: &SavedChat) -> Result<(), String> {
    if chat.archived_draft {
        return Err("An archived draft patch requires the shared document writer.".into());
    }
    chat.validate_collections()?;
    let queue = (!chat.queue.is_empty()).then_some(&chat.queue);
    write(
        dir,
        chat,
        &wire::Wire::new(chat, queue),
        !chat.queue.is_empty(),
    )
}

/// Removes the chat `id` from `dir`; a missing file is not an error.
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let path = file(dir, id).ok_or("invalid chat id")?;
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => {
            catalog::delete_cache(dir, id);
            Ok(())
        }
    }
}

/// The chats saved in `dir`, pinned first, then newest first. Unreadable
/// files are skipped.
pub fn load_all(dir: &Path) -> Vec<SavedChat> {
    let mut bytes = 0;
    load_catalog(dir)
        .into_iter()
        .filter_map(|row| {
            // Compatibility API for callers needing payloads; production uses the catalog.
            if bytes + row.bytes > 64 * 1024 * 1024 {
                return None;
            }
            let chat = load(dir, &row.id).ok()?;
            let resident = chat.resident_bytes() as u64;
            if bytes + resident > 64 * 1024 * 1024 {
                return None;
            }
            bytes += resident;
            Some(chat)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> SavedChat {
        let mut chat = SavedChat::new("codex".into());
        chat.title = Some("Disk usage".into());
        chat.session_id = Some("session-1".into());
        chat.entries = vec![
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                "df -h?",
            ))]),
            Entry::Thought("checking".into()),
            Entry::Agent("Use `df -h`.".into()),
        ];
        chat
    }

    #[test]
    fn a_chat_keeps_its_configuration_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut saved = chat();
        saved.config.insert(
            "model".into(),
            crate::session_config::ConfigValue::Value("opus".into()),
        );
        saved.mode = Some("plan".into());
        save(dir.path(), &saved).unwrap();
        let loaded = load(dir.path(), &saved.id).unwrap();
        assert_eq!(loaded.config, saved.config);
        assert_eq!(loaded.mode.as_deref(), Some("plan"));
    }

    #[test]
    fn chats_round_trip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut older = chat();
        older.updated = 10;
        let mut newer = chat();
        newer.updated = 20;
        save(dir.path(), &older).unwrap();
        save(dir.path(), &newer).unwrap();

        let loaded = load_all(dir.path());

        assert_eq!(
            loaded
                .iter()
                .map(|chat| chat.id.as_str())
                .collect::<Vec<_>>(),
            [newer.id.as_str(), older.id.as_str()]
        );
        assert_eq!(loaded[0].entries.len(), 3);
        assert!(matches!(&loaded[0].entries[2], Entry::Agent(text) if text == "Use `df -h`."));
        assert_eq!(loaded[0].session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn pinned_chats_come_first_with_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut pinned = chat();
        pinned.updated = 1;
        pinned.pinned = true;
        pinned.name = Some("Disks".into());
        let mut newer = chat();
        newer.updated = 2;
        save(dir.path(), &pinned).unwrap();
        save(dir.path(), &newer).unwrap();

        let loaded = load_all(dir.path());

        assert_eq!(loaded[0].id, pinned.id);
        assert!(loaded[0].pinned);
        assert_eq!(loaded[0].name.as_deref(), Some("Disks"));
        assert!(!loaded[1].pinned);
    }

    #[cfg(unix)]
    #[test]
    fn chats_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("chats");
        let chat = chat();
        save(&dir, &chat).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(format!("{}.json", chat.id))), 0o600);
    }

    #[test]
    fn deleting_removes_the_file_and_ids_cannot_walk_paths() {
        let dir = tempfile::tempdir().unwrap();
        let chat = chat();
        save(dir.path(), &chat).unwrap();
        delete(dir.path(), &chat.id).unwrap();
        delete(dir.path(), &chat.id).unwrap();
        assert!(load_all(dir.path()).is_empty());

        let mut evil = chat.clone();
        evil.id = "../escape".into();
        assert!(save(dir.path(), &evil).is_err());
        assert!(delete(dir.path(), "../escape").is_err());
    }

    #[test]
    fn large_images_and_old_entries_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut chat = chat();
        let image = acp::ContentBlock::Image(acp::ImageContent::new(
            "A".repeat(MAX_SAVED_IMAGE_BYTES + 1),
            "image/png",
        ));
        chat.entries = (0..MAX_SAVED_ENTRIES + 5)
            .map(|ix| Entry::Agent(ix.to_string()))
            .collect();
        chat.entries.push(Entry::User(vec![image]));
        save(dir.path(), &chat).unwrap();

        let loaded = &load_all(dir.path())[0];

        assert_eq!(loaded.entries.len(), MAX_SAVED_ENTRIES);
        assert!(matches!(
            loaded.entries.last(),
            Some(Entry::User(blocks)) if matches!(&blocks[0], acp::ContentBlock::Text(text) if text.text.contains("not kept"))
        ));
    }

    #[test]
    fn foreign_and_broken_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.json"), "{").unwrap();
        let chat = chat();
        let text = serde_json::to_string(&chat).unwrap();
        // A valid chat under someone else's name.
        std::fs::write(
            dir.path().join(format!("{}.json", uuid::Uuid::new_v4())),
            text,
        )
        .unwrap();
        assert!(load_all(dir.path()).is_empty());
    }
    #[test]
    fn pending_images_and_attachment_snapshots_round_trip_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let mut chat = chat();
        let data = "A".repeat(MAX_SAVED_IMAGE_BYTES + 1);
        chat.attachments = vec![
            SavedAttachment::Connection("server".into()),
            SavedAttachment::Group("production".into()),
        ];
        chat.queue = vec![SavedPrompt {
            id: 42,
            text: "queued image".into(),
            images: vec![acp::ContentBlock::Image(acp::ImageContent::new(
                data.clone(),
                "image/png",
            ))],
            attachments: chat.attachments.clone(),
        }];
        save(dir.path(), &chat).unwrap();
        let saved = load_all(dir.path()).remove(0);
        assert_eq!(saved.queue[0].text, "queued image");
        assert_eq!(saved.attachments, chat.attachments);
        assert_eq!(saved.queue[0].attachments, chat.attachments);
        assert!(
            matches!(&saved.queue[0].images[0], acp::ContentBlock::Image(image) if image.data == data)
        );
    }

    #[test]
    fn oversized_pending_messages_are_rejected_and_old_chats_still_load() {
        let mut chat = chat();
        let mut old = serde_json::to_value(&chat).unwrap();
        old.as_object_mut().unwrap().remove("queue");
        old.as_object_mut().unwrap().remove("attachments");
        let old: SavedChat = serde_json::from_value(old).unwrap();
        assert!(old.queue.is_empty());
        assert!(old.attachments.is_empty());
        chat.queue = vec![SavedPrompt {
            id: 1,
            text: "x".repeat(MAX_FILE_BYTES as usize),
            images: Vec::new(),
            attachments: Vec::new(),
        }];
        assert!(chat.validate_size().is_err());
    }
}

#[cfg(test)]
mod draft_tests;
