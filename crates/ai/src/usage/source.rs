//! Account usage capabilities, without native paths or provider log formats.
use super::RateLimit;
use futures::future::BoxFuture;
use nocterm_core::{SharedCause, SharedError};
use std::path::PathBuf;
use std::sync::Arc;

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
    #[error("Could not fetch account usage from {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: SharedCause,
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
    pub fn fetch(
        url: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Fetch {
            url: url.into(),
            source: SharedCause::new(source),
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

/// Several account sources, each serving the agents it recognizes: the
/// first report wins, and the first failure is returned only when no source
/// reports.
pub struct UsageSources(Vec<Arc<dyn AccountUsageSource>>);

impl UsageSources {
    pub fn new(sources: Vec<Arc<dyn AccountUsageSource>>) -> Self {
        Self(sources)
    }
}

impl AccountUsageSource for UsageSources {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>> {
        let reads: Vec<_> = self
            .0
            .iter()
            .map(|source| source.read(agent.clone()))
            .collect();
        Box::pin(async move {
            let mut failure = None;
            for read in reads {
                match read.await {
                    Ok(Some(limits)) => return Ok(Some(limits)),
                    Ok(None) => {}
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
            failure.map_or(Ok(None), Err)
        })
    }
}
