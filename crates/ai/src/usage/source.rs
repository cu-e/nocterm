//! Account usage capabilities, without native paths or provider log formats.
use super::RateLimit;
use futures::future::BoxFuture;
use nocterm_core::SharedError;
use std::path::PathBuf;

/// The launch identity needed to select an account source; environment is omitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageAgent {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
}

impl From<&crate::AgentLaunch> for UsageAgent {
    fn from(launch: &crate::AgentLaunch) -> Self {
        Self {
            id: launch.id.clone(),
            command: launch.command.clone(),
            args: launch.args.clone(),
        }
    }
}

/// Physical account-source failures retain their native cause.
#[derive(Clone, Debug, thiserror::Error)]
pub enum UsageReadError {
    #[error("Could not enumerate account usage at {path}: {source}")]
    Enumerate {
        path: PathBuf,
        #[source]
        source: SharedError<std::io::Error>,
    },
    #[error("Could not read account usage at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: SharedError<std::io::Error>,
    },
    #[error("Could not seek account usage at {path}: {source}")]
    Seek {
        path: PathBuf,
        #[source]
        source: SharedError<std::io::Error>,
    },
}

impl UsageReadError {
    pub fn enumerate(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Enumerate {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
    pub fn read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Read {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
    pub fn seek(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Seek {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
}

/// Implementations read only when polled on the host's background executor.
/// Missing or unsupported sources and logs without a usable report return None.
pub trait AccountUsageSource: Send + Sync {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>>;
}
