//! Immutable chat publication; the service supplies serialization order.
use crate::{catalog, loading, writing};
#[cfg(test)]
use crate::{format, legacy};
#[cfg(test)]
use nocterm_ai::history::{ChatWrite, HistoryEncoding, SavedChat, SharedChat};
use nocterm_ai::{
    history::{self, ChatSummary},
    storage::{AgentStorageError, ChatRepository, StorageFuture},
};
use nocterm_core::SharedError;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone)]
pub struct FileChatRepository {
    dir: Arc<PathBuf>,
    #[cfg(test)]
    retention: Option<nocterm_ai::model::retention::RetentionAuthority>,
}
impl FileChatRepository {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir: Arc::new(dir),
            #[cfg(test)]
            retention: None,
        }
    }
}
/// Legacy whole-document routes remain only as original oracles for tests.
#[cfg(test)]
impl FileChatRepository {
    pub(crate) fn with_retention(
        dir: PathBuf,
        retention: nocterm_ai::model::retention::RetentionAuthority,
    ) -> Self {
        Self {
            dir: Arc::new(dir),
            retention: Some(retention),
        }
    }
    fn retention(&self) -> nocterm_ai::model::retention::RetentionAuthority {
        self.retention.clone().expect("legacy oracle retention")
    }
    pub(crate) fn load(&self, id: String) -> StorageFuture<SavedChat> {
        let dir = self.dir.clone();
        let retention = self.retention();
        Box::pin(async move { load(&dir, &id, &retention) })
    }
    pub(crate) fn load_into(
        &self,
        id: String,
        request: history::ReaderRequest,
    ) -> futures::future::BoxFuture<'static, history::ReaderDelivery> {
        // The repository's immutable directory owner is shared without allocating
        // a new PathBuf before the native phase can admit its actual scratch.
        let dir = self.dir.clone();
        Box::pin(async move { loading::native::read_request(&dir, &id, request) })
    }
    pub(crate) fn save(&self, chat: Arc<ChatWrite>) -> StorageFuture<()> {
        let dir = self.dir.clone();
        let retention = self.retention();
        Box::pin(async move { save_write(&dir, &chat, &retention) })
    }
}
impl ChatRepository for FileChatRepository {
    fn load_managed(
        &self,
        id: history::ReaderDocumentId,
        request: history::ReaderRequest,
    ) -> Result<history::ManagedReaderFuture, history::ReaderPreparationFailure> {
        loading::reader_task::prepare(self.dir.clone(), id, request)
    }
    fn catalog(&self) -> StorageFuture<Vec<ChatSummary>> {
        let dir = self.dir.clone();
        Box::pin(async move { Ok(catalog::load_catalog(&dir)) })
    }
    fn save_owned(&self, write: history::writing::ManagedWrite) -> StorageFuture<()> {
        let dir = self.dir.clone();
        Box::pin(async move { writing::managed::save(&dir, write) })
    }
    fn delete(&self, id: String) -> StorageFuture<()> {
        let dir = self.dir.clone();
        Box::pin(async move { delete(&dir, &id) })
    }
}
#[cfg(test)]
pub(super) fn load(
    dir: &Path,
    id: &str,
    retention: &nocterm_ai::model::retention::RetentionAuthority,
) -> Result<SavedChat, AgentStorageError> {
    let path = loading::file(dir, id)?;
    let bytes = loading::read(&path, id)?;
    let chat = match format::classify(&bytes)? {
        format::Format::Version2 => history::decode(retention, &bytes, id)?,
        format::Format::LegacyV1 => legacy::decode(retention, &bytes, id)?,
    };
    Ok(chat.describe_imported_rows(legacy::describe_row))
}
#[cfg(test)]
pub(super) fn save(dir: &Path, chat: &SavedChat) -> Result<(), AgentStorageError> {
    save_shared(dir, &SharedChat::from_saved(chat.clone())?)
}
#[cfg(test)]
pub(super) fn save_write(
    dir: &Path,
    write: &ChatWrite,
    retention: &nocterm_ai::model::retention::RetentionAuthority,
) -> Result<(), AgentStorageError> {
    match write {
        ChatWrite::Snapshot(chat) => save_shared(dir, chat),
        ChatWrite::ArchiveDraft(patch) => {
            let original = load(dir, &patch.id, retention)?;
            // A no-op public draft intent cannot rewrite an untouched v1 file
            // or update its timestamp merely because the UI was restored.
            if original.composer().draft == patch.text {
                return Ok(());
            }
            let modified = original.with_archived_draft(patch)?;
            save_shared(dir, &SharedChat::from_saved(modified)?)
        }
    }
}
#[cfg(test)]
pub(super) fn save_shared(dir: &Path, chat: &SharedChat) -> Result<(), AgentStorageError> {
    let decision = chat.decision()?;
    history::verify_selected(chat, decision)?;
    let path = loading::file(dir, chat.snapshot().id())?;
    nocterm_core::paths::ensure_private_dir(dir).map_err(|source| {
        AgentStorageError::Directory {
            path: dir.to_owned(),
            source: SharedError::from_error(source),
        }
    })?;
    let staged = match decision.encoding {
        HistoryEncoding::Version2 => writing::stage(
            &path,
            &history::projection_shared(chat, decision)?,
            decision,
        ),
        HistoryEncoding::LegacyV1 => writing::stage(
            &path,
            &legacy::projection(
                chat.view()?,
                decision,
                chat.snapshot()
                    .prepared()
                    .and_then(history::PreparedHistory::legacy)
                    .ok_or(history::HistoryCodecError::CandidateMismatch)?
                    .as_ref(),
            )?,
            decision,
        ),
    }
    .map_err(|error| match error {
        nocterm_core::fs::StageError::Native(source) => AgentStorageError::Publish {
            path: path.clone(),
            source: SharedError::from_error(source),
        },
        nocterm_core::fs::StageError::Producer(writing::ProducerError::Io(source)) => {
            AgentStorageError::Publish {
                path: path.clone(),
                source: SharedError::from_error(nocterm_core::fs::FileError::Io {
                    path: path.clone(),
                    source,
                }),
            }
        }
        nocterm_core::fs::StageError::Producer(writing::ProducerError::Codec(error)) => {
            AgentStorageError::Codec(error)
        }
    })?;
    staged
        .publish()
        .map_err(|source| AgentStorageError::Publish {
            path,
            source: SharedError::from_error(source),
        })?;
    catalog::update_cache(dir, chat.snapshot(), !chat.prompts().is_empty());
    Ok(())
}
pub(super) fn delete(dir: &Path, id: &str) -> Result<(), AgentStorageError> {
    let path = loading::file(dir, id)?;
    match std::fs::remove_file(&path) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
            Err(AgentStorageError::Delete {
                path,
                source: SharedError::from_error(source),
            })
        }
        _ => {
            catalog::delete_cache(dir, id);
            Ok(())
        }
    }
}
#[cfg(test)]
fn load_all(
    dir: &Path,
    authority: &nocterm_ai::model::retention::RetentionAuthority,
) -> Vec<SavedChat> {
    let mut bytes = 0;
    catalog::load_catalog(dir)
        .into_iter()
        .filter_map(|row| {
            if bytes + row.bytes > 64 * 1024 * 1024 {
                return None;
            }
            let chat = load(dir, &row.id, authority).ok()?;
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
