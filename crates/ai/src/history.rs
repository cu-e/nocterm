//! Saved chats: one JSON file per chat in a private directory.
//!
//! A chat keeps what the panel showed — the user's messages, the agent's
//! replies and tool calls — and the agent session it belonged to, so it can be
//! reopened after a restart. The terminal context sent with each prompt is
//! never part of the transcript, so it is never saved.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{acp, thread::Entry};

mod budget;
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
    #[serde(default)]
    pub images: Vec<acp::ContentBlock>,
    #[serde(default)]
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
    /// Saved transcript still needs delivery to the current agent session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending_history: bool,
    /// Seconds since the Unix epoch.
    pub updated: u64,
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<SavedPrompt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SavedAttachment>,
    /// When each entry began, aligned with `entries`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub times: Vec<u64>,
    #[serde(skip)]
    prepared: Option<PreparedBudget>,
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
            pending_history: false,
            updated: now(),
            entries: Vec::new(),
            queue: Vec::new(),
            attachments: Vec::new(),
            times: Vec::new(),
            prepared: None,
        }
    }

    /// Reject a prompt before accepting it if it could not be read back.
    pub fn validate_size(&self) -> Result<(), String> {
        let queue = (!self.queue.is_empty()).then_some(&self.queue);
        check_size(
            size::encoded_len(&wire::Wire::new(self, queue)).map_err(|error| error.to_string())?,
        )
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
    let prompts = wire::Prompts(&chat.queue);
    let queue = (!chat.queue.is_empty()).then_some(&prompts);
    write(dir, &chat.metadata, &wire::Wire::new(&chat.metadata, queue))
}
fn write(dir: &Path, chat: &SavedChat, value: &impl Serialize) -> Result<(), String> {
    let path = file(dir, &chat.id).ok_or("invalid chat id")?;
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    check_size(text.len())?;
    nocterm_core::paths::ensure_private_dir(dir).map_err(|error| error.to_string())?;
    nocterm_core::persist::save_text(&path, &text).map_err(|error| error.to_string())
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
    let queue = (!chat.queue.is_empty()).then_some(&chat.queue);
    write(dir, chat, &wire::Wire::new(chat, queue))
}

/// Removes the chat `id` from `dir`; a missing file is not an error.
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let path = file(dir, id).ok_or("invalid chat id")?;
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// The chats saved in `dir`, pinned first, then newest first. Unreadable
/// files are skipped.
pub fn load_all(dir: &Path) -> Vec<SavedChat> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut chats: Vec<SavedChat> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
                && entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_FILE_BYTES)
        })
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.path()).ok()?;
            let mut chat: SavedChat = serde_json::from_str(&text).ok()?;
            chat.prepared = Some(PreparedBudget::new(&chat).ok()?);
            let name_matches = entry
                .path()
                .file_stem()
                .is_some_and(|stem| *stem == *chat.id);
            (chat.version == VERSION && valid_id(&chat.id) && name_matches).then_some(chat)
        })
        .collect();
    chats.sort_by_key(|chat| std::cmp::Reverse((chat.pinned, chat.updated)));
    chats.truncate(MAX_RESTORED);
    chats
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
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
