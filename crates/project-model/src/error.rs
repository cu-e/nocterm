use std::{error::Error, path::PathBuf};

use nocterm_core::{SharedCause, SharedError};

use crate::ProjectId;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectValidationError {
    #[error("choose a non-empty project name of at most 128 bytes")]
    InvalidName,
    #[error("choose an unambiguous absolute POSIX project directory")]
    InvalidRoot,
    #[error("the project host or jump route is invalid")]
    InvalidHost,
    #[error("the imported source identity is invalid")]
    InvalidSource,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("project catalog exceeds its byte limit")]
    TooLarge,
    #[error("project catalog contains too many definitions")]
    Capacity,
    #[error("project catalog version {0} is unsupported")]
    UnsupportedVersion(u32),
    #[error("project {0} occurs more than once")]
    DuplicateId(ProjectId),
    #[error("project {0} no longer exists")]
    Missing(ProjectId),
    #[error("project {0} changed before this operation completed")]
    Conflict(ProjectId),
    #[error("project revision cannot advance further")]
    RevisionExhausted,
    #[error(transparent)]
    Invalid(#[from] ProjectValidationError),
    #[error("project catalog JSON is invalid")]
    Codec(#[source] SharedError<serde_json::Error>),
}

impl From<serde_json::Error> for CatalogError {
    fn from(error: serde_json::Error) -> Self {
        Self::Codec(SharedError::from_error(error))
    }
}

/// Storage diagnostics preserve original native causes without I/O domain APIs.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectStorageError {
    #[error(transparent)]
    InvalidDocument(#[from] CatalogError),
    #[error("could not read project catalog {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: SharedCause,
    },
    #[error("could not publish project catalog {}: {source}", path.display())]
    Publish {
        path: PathBuf,
        #[source]
        source: SharedCause,
    },
}

impl ProjectStorageError {
    pub fn read(path: PathBuf, source: impl Error + Send + Sync + 'static) -> Self {
        Self::Read {
            path,
            source: SharedCause::new(source),
        }
    }

    pub fn publish(path: PathBuf, source: impl Error + Send + Sync + 'static) -> Self {
        Self::Publish {
            path,
            source: SharedCause::new(source),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectMetadataError {
    #[error("native project identity entropy is unavailable")]
    Entropy(#[source] SharedCause),
    #[error("the supplied project identity sequence is exhausted")]
    Exhausted,
}

impl ProjectMetadataError {
    pub fn entropy(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Entropy(SharedCause::new(source))
    }
}
