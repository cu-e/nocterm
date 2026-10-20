//! Native workspace preparation is supplied by an adapter, never performed here.
use crate::sandbox::{ProtectedWorkdirError, SandboxPolicy};
use futures::future::BoxFuture;
use nocterm_core::SharedError;
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsolationPaths {
    pub private_dirs: Vec<PathBuf>,
    pub shared_dirs: Vec<PathBuf>,
    /// Agent state the sandbox keeps writable, relative to the home directory.
    pub home_state: Vec<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkdirRequest {
    pub candidate: PathBuf,
    pub create_private: bool,
    pub isolation: Option<IsolationPaths>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedWorkspace {
    pub root: PathBuf,
    pub sandbox: Option<SandboxPolicy>,
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum WorkdirError {
    #[error("Working directory must be an existing absolute directory.")]
    InvalidDirectory,
    #[error("Could not create agent workspace at {path}: {source}")]
    CreatePrivate {
        path: PathBuf,
        #[source]
        source: SharedError<std::io::Error>,
    },
    #[error("Could not resolve agent workspace path {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: SharedError<std::io::Error>,
    },
    #[error("{0}")]
    Protected(#[from] ProtectedWorkdirError),
    #[error("The agent working directory changed during startup. Retry opening the chat.")]
    ChangedSincePrepared,
}
impl WorkdirError {
    pub fn create_private(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::CreatePrivate {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
    pub fn resolve(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Resolve {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
}
pub trait WorkdirResolver: Send + Sync {
    fn prepare(
        &self,
        request: WorkdirRequest,
    ) -> BoxFuture<'static, Result<PreparedWorkspace, WorkdirError>>;
}
