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

const VERSION: u32 = 1;
/// Images larger than this are left out of saved chats.
const MAX_SAVED_IMAGE_BYTES: usize = 512 * 1024;
/// Only the most recent entries of a very long chat are kept.
const MAX_SAVED_ENTRIES: usize = 2_000;
/// Larger files are not read back.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// At most this many chats are restored, newest first.
pub const MAX_RESTORED: usize = 200;

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
    /// Seconds since the Unix epoch.
    pub updated: u64,
    pub entries: Vec<Entry>,
    /// When each entry began, aligned with `entries`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub times: Vec<u64>,
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
            updated: now(),
            entries: Vec::new(),
            times: Vec::new(),
        }
    }

    /// The chat as it is written: bounded, with large images left out.
    fn bounded(&self) -> Self {
        let skip = self.entries.len().saturating_sub(MAX_SAVED_ENTRIES);
        let entries = self.entries[skip..]
            .iter()
            .map(|entry| match entry {
                Entry::User(blocks) => Entry::User(blocks.iter().map(bounded_block).collect()),
                Entry::Content(block) => Entry::Content(bounded_block(block)),
                entry => entry.clone(),
            })
            .collect();
        Self {
            entries,
            times: self.times.iter().skip(skip).copied().collect(),
            ..self.clone()
        }
    }
}

fn bounded_block(block: &acp::ContentBlock) -> acp::ContentBlock {
    match block {
        acp::ContentBlock::Image(image) if image.data.len() > MAX_SAVED_IMAGE_BYTES => {
            acp::ContentBlock::Text(acp::TextContent::new("[Image not kept in history]"))
        }
        block => block.clone(),
    }
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
    let path = file(dir, &chat.id).ok_or("invalid chat id")?;
    nocterm_core::paths::ensure_private_dir(dir).map_err(|error| error.to_string())?;
    let text = serde_json::to_string(&chat.bounded()).map_err(|error| error.to_string())?;
    nocterm_core::persist::save_text(&path, &text).map_err(|error| error.to_string())
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
            let chat: SavedChat = serde_json::from_str(&text).ok()?;
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
}
