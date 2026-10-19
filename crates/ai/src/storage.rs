//! Narrow storage contracts. Implementations are composed by the application.
use crate::{
    favorites::AgentStateFile,
    history::{ChatSummary, HistoryCodecError},
};
mod label;
use futures::future::BoxFuture;
pub use label::{ManagedLabelFailure, StorageLabel};
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum ManagedPreparationCause {
    #[error(transparent)]
    Native(#[from] nocterm_core::fs::CurrentDirectoryError),
    #[error(transparent)]
    Admission(#[from] crate::model::json::deferred::JsonDecodeError),
}
pub type ManagedPathFailure = nocterm_core::SharedResidentError<
    ManagedPreparationCause,
    crate::model::json::deferred::NativeScratch,
>;

pub type ManagedFileFailure = nocterm_core::SharedResidentError<
    nocterm_core::fs::AdmittedFileError<crate::model::json::deferred::NativeScratch>,
    crate::model::json::deferred::NativeScratch,
>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentStorageError {
    #[error(transparent)]
    ManagedReplayRead(crate::model::failure::ManagedNativeFailure),
    #[error("Could not read {path}: {cause}", path = .0.as_ref().path().display(), cause = .0.as_ref().native_source())]
    ManagedRead(#[source] ManagedFileFailure),
    #[error("Saved chat was not found: {id}", id = .0.as_ref().path().file_stem().unwrap_or_default().to_string_lossy())]
    ManagedNotFound(#[source] ManagedFileFailure),
    #[error("Invalid chat identity: {0}")]
    ManagedInvalidId(#[source] ManagedLabelFailure),
    #[error("Storage allocation admission failed: {0}")]
    ManagedAdmission(#[source] crate::model::json::deferred::NativeAllocationError),
    #[error(transparent)]
    ManagedPathPreparation(ManagedPathFailure),
    #[error("Could not publish {path}: {0}", path = .0.as_ref().path().display())]
    ManagedPublication(#[source] ManagedFileFailure),
    #[error("Could not create private directory {path}: {cause}", path = .0.as_ref().path().display(), cause = .0.as_ref().native_source())]
    ManagedDirectory(#[source] ManagedFileFailure),
    #[error("This repository does not support managed history publication")]
    ManagedWriteUnsupported,
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
impl AgentStorageError {
    pub fn managed_invalid_id(
        ledger: &crate::model::json::deferred::DecodeLedger,
        id: &str,
    ) -> Result<Self, crate::model::json::deferred::NativeAllocationError> {
        Ok(Self::ManagedInvalidId(StorageLabel::prepare(ledger, id)?))
    }
    pub fn invalid_identity(&self) -> Option<&str> {
        match self {
            Self::InvalidId(id) => Some(id),
            Self::ManagedInvalidId(id) => Some(id.as_ref().as_str()),
            _ => None,
        }
    }
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_) | Self::ManagedNotFound(_))
    }
}
pub type StorageFuture<T> = BoxFuture<'static, Result<T, AgentStorageError>>;
/// Futures perform I/O only when polled by the background storage executor.
pub trait AgentStateRepository: Send + Sync {
    fn load(&self) -> StorageFuture<AgentStateFile>;
    fn save(&self, state: AgentStateFile) -> StorageFuture<()>;
}
pub trait ChatRepository: Send + Sync {
    /// By-value original-reader operation; task admission precedes its native Box.
    #[expect(
        clippy::result_large_err,
        reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
    )]
    fn load_managed(
        &self,
        id: crate::history::ReaderDocumentId,
        request: crate::history::ReaderRequest,
    ) -> Result<crate::history::ManagedReaderFuture, crate::history::ReaderPreparationFailure>;
    fn catalog(&self) -> StorageFuture<Vec<ChatSummary>>;
    /// The physical future owns the original writer loan through publication.
    /// Transitional implementations fail explicitly without exporting a DTO.
    fn save_owned(&self, write: crate::history::writing::ManagedWrite) -> StorageFuture<()> {
        Box::pin(async move {
            drop(write);
            Err(AgentStorageError::ManagedWriteUnsupported)
        })
    }
    fn delete(&self, id: String) -> StorageFuture<()>;
}
