//! Account usage capabilities, without native paths or provider log formats.
use super::RateLimit;
use futures::future::BoxFuture;
use std::{path::PathBuf, sync::Arc};

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
        source: Arc<std::io::Error>,
    },
    #[error("Could not read account usage at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
    #[error("Could not seek account usage at {path}: {source}")]
    Seek {
        path: PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
}

/// Implementations read only when polled on the host's background executor.
/// Missing or unsupported sources and logs without a usable report return None.
pub trait AccountUsageSource: Send + Sync {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>>;
}
