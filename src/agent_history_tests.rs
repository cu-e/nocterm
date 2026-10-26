//! Native repository composition: domain codecs and held local filesystem ports.
use futures::executor::block_on;
use nocterm_agent_storage::FileChatRepository;
use nocterm_ai::{
    history::{self, ContextId, MAX_RESTORED, SavedChat, SharedChat},
    storage::{AgentStorageError, ChatRepository},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
mod catalog;
mod namespaces;
mod transcripts;
fn repository(dir: &Path) -> FileChatRepository {
    FileChatRepository::new(
        dir.to_owned(),
        Arc::new(nocterm_localfs::LocalFilesystem::new().unwrap()),
    )
}
fn document(dir: &Path, scope: ContextId, id: &str, extension: &str) -> PathBuf {
    let partition = match scope {
        ContextId::General => dir.join("scoped-v2/general"),
        ContextId::Project(id) => dir.join("scoped-v2/projects").join(id.to_string()),
    };
    partition.join(format!("{id}.{extension}"))
}
fn save(dir: &Path, chat: &SavedChat) -> Result<(), AgentStorageError> {
    save_shared(
        dir,
        &SharedChat {
            metadata: chat.clone(),
            queue: chat.queue.iter().cloned().map(Arc::new).collect(),
        },
    )
}
fn save_shared(dir: &Path, chat: &SharedChat) -> Result<(), AgentStorageError> {
    block_on(repository(dir).save(Arc::new(SharedChat {
        metadata: chat.metadata.clone(),
        queue: chat.queue.clone(),
    })))
}
fn load(dir: &Path, id: &str) -> Result<SavedChat, AgentStorageError> {
    load_scoped(dir, ContextId::General, id)
}
fn load_scoped(dir: &Path, scope: ContextId, id: &str) -> Result<SavedChat, AgentStorageError> {
    block_on(repository(dir).load(scope, id.to_owned()))
}
fn delete(dir: &Path, id: &str) -> Result<(), AgentStorageError> {
    block_on(repository(dir).delete(ContextId::General, id.to_owned()))
}
fn load_catalog(dir: &Path, scope: ContextId) -> Vec<history::ChatSummary> {
    block_on(repository(dir).catalog(scope)).unwrap()
}
fn load_all(dir: &Path) -> Vec<SavedChat> {
    load_catalog(dir, ContextId::General)
        .iter()
        .map(|row| load(dir, &row.id).unwrap())
        .collect()
}
