//! Narrow storage contracts. Implementations are composed by the application.
use crate::{
    favorites::AgentStateFile,
    history::{ChatSummary, HistoryCodecError, SavedChat, SharedChat},
};
use futures::future::BoxFuture;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentStorageError {
    #[error("Storage operation was interrupted before completion")]
    Interrupted,
    #[error("Invalid chat identity: {0}")]
    InvalidId(String),
    #[error("Saved chat was not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Codec(#[from] HistoryCodecError),
    #[error("Could not decode agent state: {0}")]
    StateDecode(#[source] Arc<toml::de::Error>),
    #[error("Could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
    #[error("Could not publish {path}: {source}")]
    Publish {
        path: PathBuf,
        #[source]
        source: Arc<nocterm_core::fs::FileError>,
    },
    #[error("Could not create private directory {path}: {source}")]
    Directory {
        path: PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
    #[error("Could not save agent state at {path}: {source}")]
    StatePublication {
        path: PathBuf,
        #[source]
        source: Arc<nocterm_core::persist::PersistError>,
    },
    #[error("Could not delete {path}: {source}")]
    Delete {
        path: PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
}
pub type StorageFuture<T> = BoxFuture<'static, Result<T, AgentStorageError>>;
/// Futures perform I/O only when polled by the background storage executor.
pub trait AgentStateRepository: Send + Sync {
    fn load(&self) -> StorageFuture<AgentStateFile>;
    fn save(&self, state: AgentStateFile) -> StorageFuture<()>;
}
pub trait ChatRepository: Send + Sync {
    fn catalog(&self) -> StorageFuture<Vec<ChatSummary>>;
    fn load(&self, id: String) -> StorageFuture<SavedChat>;
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()>;
    fn delete(&self, id: String) -> StorageFuture<()>;
}
