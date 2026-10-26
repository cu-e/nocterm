//! Narrow storage contracts. Implementations are composed by the application.
use crate::{
    favorites::AgentStateFile,
    history::{ChatSummary, HistoryCodecError, SavedChat, SharedChat},
};
use futures::future::BoxFuture;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentStorageError {
    #[error(
        "Saved history needs {incoming} bytes with {retained} retained; installation limit is {limit} bytes"
    )]
    ResidentCapacity {
        retained: usize,
        incoming: usize,
        limit: usize,
    },
    #[error("Storage operation was interrupted before completion")]
    Interrupted,
    #[error("Invalid chat identity: {0}")]
    InvalidId(String),
    #[error("Saved chat was not found: {0}")]
    NotFound(String),
    #[error(
        "Saved chat ownership differs from the requested context: expected {expected:?}, found {actual:?}"
    )]
    ScopeMismatch {
        expected: nocterm_project_model::ContextId,
        actual: nocterm_project_model::ContextId,
    },
    #[error("This context requires held native storage directory capabilities: {0:?}")]
    ScopeUnsupported(nocterm_project_model::ContextId),
    #[error("The local history filesystem operation failed: {0}")]
    Filesystem(#[source] nocterm_core::SharedError<nocterm_session::local_fs::LocalFsError>),
    #[error("History publication failed: {operation}; staging cleanup also failed: {cleanup}")]
    StageCleanup {
        #[source]
        operation: nocterm_core::SharedError<nocterm_session::local_fs::LocalFsError>,
        cleanup: nocterm_core::SharedError<nocterm_session::local_fs::LocalFsError>,
    },
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
impl From<nocterm_session::local_fs::LocalFsError> for AgentStorageError {
    fn from(error: nocterm_session::local_fs::LocalFsError) -> Self {
        Self::Filesystem(nocterm_core::SharedError::from_error(error))
    }
}
/// Check descriptive document ownership before admitting it to a bound consumer.
/// Matching identities do not issue execution capabilities.
pub fn require_chat_scope(
    expected: nocterm_project_model::ContextId,
    actual: nocterm_project_model::ContextId,
) -> Result<(), AgentStorageError> {
    if expected != actual {
        return Err(AgentStorageError::ScopeMismatch { expected, actual });
    }
    Ok(())
}
pub type StorageFuture<T> = BoxFuture<'static, Result<T, AgentStorageError>>;
/// Futures perform I/O only when polled by the background storage executor.
pub trait AgentStateRepository: Send + Sync {
    fn load(&self) -> StorageFuture<AgentStateFile>;
    fn save(&self, state: AgentStateFile) -> StorageFuture<()>;
}
pub trait ChatRepository: Send + Sync {
    /// Filter authoritative ownership before applying per-context discovery limits.
    fn catalog(&self, scope: nocterm_project_model::ContextId) -> StorageFuture<Vec<ChatSummary>>;
    fn load(&self, scope: nocterm_project_model::ContextId, id: String)
    -> StorageFuture<SavedChat>;
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()>;
    fn delete(&self, scope: nocterm_project_model::ContextId, id: String) -> StorageFuture<()>;
}
