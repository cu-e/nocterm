//! Remote OS acquisition capability with typed, transport-independent causes.
use crate::os::OsId;
use nocterm_session::{FsError, RemoteFs};
use std::{future::Future, pin::Pin, sync::Arc};
pub type OsFuture = Pin<Box<dyn Future<Output = Result<Option<OsId>, OsObservationError>> + Send>>;
pub trait RemoteOsDetector: Send + Sync {
    fn detect(&self, fs: Arc<dyn RemoteFs>) -> OsFuture;
}
#[derive(Debug, thiserror::Error)]
pub enum OsObservationError {
    #[error("could not download {path}: {source}")]
    Download {
        path: String,
        #[source]
        source: FsError,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: FsError,
    },
    #[error("could not inspect {path}: {source}")]
    Stat {
        path: String,
        #[source]
        source: FsError,
    },
    #[error("could not read home: {0}")]
    Home(#[source] FsError),
    #[error("could not close {path}: {source}")]
    Close {
        path: String,
        #[source]
        source: FsError,
    },
    #[error("{path}: invalid UTF-8: {source}")]
    InvalidUtf8 {
        path: String,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("{path}: OS release exceeds {limit} bytes")]
    Oversize { path: String, limit: usize },
    #[error("{path}: reader returned {returned} bytes when requested {requested}")]
    InvalidChunk {
        path: String,
        requested: usize,
        returned: usize,
    },
}
/// Inert composition performs no remote requests.
pub struct OfflineOsDetector;
impl RemoteOsDetector for OfflineOsDetector {
    fn detect(&self, _: Arc<dyn RemoteFs>) -> OsFuture {
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests;
