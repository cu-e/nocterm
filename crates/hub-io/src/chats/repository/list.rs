//! The chats of one hub as this device last read them, kept in a private file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubId;
use serde::{Deserialize, Serialize};

/// A list larger than this is not ours.
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
/// The format the file is written in; another one is read as empty.
const FORMAT: u32 = 1;

/// One hub chat as the history lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Row {
    pub(super) id: String,
    pub(super) agent_id: String,
    pub(super) agent_name: String,
    pub(super) title: String,
    pub(super) pinned: bool,
    updated_at_ms: i64,
}

impl Row {
    pub(super) fn of(chat: &v2::Chat) -> Self {
        Self {
            id: chat.id.clone(),
            agent_id: chat.agent_id.clone(),
            agent_name: chat.agent_name.clone(),
            title: chat.title.clone(),
            pinned: chat.pinned,
            updated_at_ms: chat.updated_at_ms,
        }
    }

    /// Seconds since the Unix epoch, as the history sorts by.
    pub(super) fn updated(&self) -> u64 {
        u64::try_from(self.updated_at_ms / 1000).unwrap_or(0)
    }
}

/// The hub's chats, and the hub chat behind each chat started here under its own id.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct ChatList {
    v: u32,
    rows: Vec<Row>,
    /// Local id of a chat started on this device → its hub chat.
    started_here: BTreeMap<String, String>,
}

fn path(root: &Path, hub: &HubId) -> PathBuf {
    let stem = crate::credentials::file_stem(hub.address());
    root.join(format!("{stem}.chats.json"))
}

impl ChatList {
    /// The list kept for `hub`, or an empty one when none is kept or it cannot be read.
    pub(super) fn load(root: &Path, hub: &HubId) -> Self {
        let read = nocterm_core::fs::read_regular_bounded(&path(root, hub), MAX_FILE_BYTES);
        let Ok(bytes) = read else {
            return Self::default();
        };
        serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(|list| list.v == FORMAT)
            .unwrap_or_default()
    }

    /// Keeps the list; a failure is logged, the list still works in memory.
    pub(super) fn save(&self, root: &Path, hub: &HubId) {
        let saved = Self {
            v: FORMAT,
            rows: self.rows.clone(),
            started_here: self.started_here.clone(),
        };
        let written = serde_json::to_vec(&saved)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                nocterm_core::fs::write_private_atomic(&path(root, hub), &bytes)
                    .map_err(std::io::Error::other)
            });
        if let Err(error) = written {
            tracing::warn!(%error, "the hub's chat list cannot be kept on this device");
        }
    }

    /// Replaces the rows with what the hub listed; `true` when they changed. Chats started
    /// here that the hub no longer lists are forgotten.
    pub(super) fn replace(&mut self, rows: Vec<Row>) -> bool {
        if rows == self.rows {
            return false;
        }
        self.started_here
            .retain(|_, hub_chat| rows.iter().any(|row| row.id == *hub_chat));
        self.rows = rows;
        true
    }

    /// Notes that the chat `local` started here is the hub chat `hub_chat`.
    pub(super) fn remember(&mut self, local: &str, hub_chat: &str) {
        if local != hub_chat && !self.started_here.contains_key(local) {
            self.started_here
                .insert(local.to_owned(), hub_chat.to_owned());
        }
    }

    /// The ids the history lists the hub chats by.
    pub(super) fn local_ids(&self) -> impl Iterator<Item = String> + '_ {
        self.rows.iter().map(|row| {
            self.started_here
                .iter()
                .find(|(_, hub_chat)| **hub_chat == row.id)
                .map_or_else(|| row.id.clone(), |(local, _)| local.clone())
        })
    }

    /// The hub chat a history id stands for.
    pub(super) fn row_for(&self, id: &str) -> Option<&Row> {
        let hub_chat = self.started_here.get(id).map_or(id, String::as_str);
        self.rows.iter().find(|row| row.id == hub_chat)
    }

    /// Shows a rename or pin before the hub confirms it.
    pub(super) fn apply(&mut self, hub_chat: &str, title: Option<&str>, pinned: Option<bool>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == hub_chat) {
            if let Some(title) = title {
                title.clone_into(&mut row.title);
            }
            if let Some(pinned) = pinned {
                row.pinned = pinned;
            }
        }
    }

    /// Forgets the chat a history id stands for and answers its hub chat.
    pub(super) fn forget(&mut self, id: &str) -> Option<String> {
        let hub_chat = self.row_for(id)?.id.clone();
        self.rows.retain(|row| row.id != hub_chat);
        self.started_here.retain(|_, chat| *chat != hub_chat);
        Some(hub_chat)
    }
}
