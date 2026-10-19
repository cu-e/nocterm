//! Immutable chat publication; the service supplies serialization order.
use crate::{catalog, loading};
use nocterm_ai::{
    history::{self, ChatSummary, SavedChat, SharedChat},
    storage::{AgentStorageError, ChatRepository, StorageFuture},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone)]
pub struct FileChatRepository {
    dir: PathBuf,
}
impl FileChatRepository {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}
impl ChatRepository for FileChatRepository {
    fn catalog(&self) -> StorageFuture<Vec<ChatSummary>> {
        let dir = self.dir.clone();
        Box::pin(async move { Ok(catalog::load_catalog(&dir)) })
    }
    fn load(&self, id: String) -> StorageFuture<SavedChat> {
        let dir = self.dir.clone();
        Box::pin(async move { load(&dir, &id) })
    }
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()> {
        let dir = self.dir.clone();
        Box::pin(async move { save_shared(&dir, &chat) })
    }
    fn delete(&self, id: String) -> StorageFuture<()> {
        let dir = self.dir.clone();
        Box::pin(async move { delete(&dir, &id) })
    }
}
pub(super) fn load(dir: &Path, id: &str) -> Result<SavedChat, AgentStorageError> {
    let path = loading::file(dir, id)?;
    Ok(history::decode(&loading::read(&path, id)?, id)?)
}
pub(super) fn save(dir: &Path, chat: &SavedChat) -> Result<(), AgentStorageError> {
    publish(dir, chat, history::encode(chat)?, !chat.queue.is_empty())
}
pub(super) fn save_shared(dir: &Path, chat: &SharedChat) -> Result<(), AgentStorageError> {
    if let Some((draft, updated)) = chat.metadata.draft_patch() {
        let mut original = load(dir, &chat.id)?;
        original.draft = draft;
        original.updated = updated;
        return save(dir, &original);
    }
    publish(
        dir,
        &chat.metadata,
        history::encode_shared(chat)?,
        !chat.queue.is_empty(),
    )
}
fn publish(
    dir: &Path,
    chat: &SavedChat,
    bytes: Vec<u8>,
    queued: bool,
) -> Result<(), AgentStorageError> {
    let path = loading::file(dir, &chat.id)?;
    nocterm_core::paths::ensure_private_dir(dir).map_err(|source| {
        AgentStorageError::Directory {
            path: dir.to_owned(),
            source: Arc::new(source),
        }
    })?;
    nocterm_core::fs::write_private_atomic(&path, &bytes).map_err(|source| {
        AgentStorageError::Publish {
            path,
            source: Arc::new(source),
        }
    })?;
    catalog::update_cache(dir, chat, queued);
    Ok(())
}
pub(super) fn delete(dir: &Path, id: &str) -> Result<(), AgentStorageError> {
    let path = loading::file(dir, id)?;
    match std::fs::remove_file(&path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            Err(AgentStorageError::Delete {
                path,
                source: Arc::new(source),
            })
        }
        _ => {
            catalog::delete_cache(dir, id);
            Ok(())
        }
    }
}
#[cfg(test)]
fn load_all(dir: &Path) -> Vec<SavedChat> {
    let mut bytes = 0;
    catalog::load_catalog(dir)
        .into_iter()
        .filter_map(|row| {
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
#[path = "history_tests.rs"]
mod tests;
