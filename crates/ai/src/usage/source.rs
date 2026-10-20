//! Account usage capabilities, without native paths or provider log formats.
use super::RateLimit;
use crate::agents::AccountUsage;
use futures::future::BoxFuture;
use nocterm_core::{SharedCause, SharedError};
use std::path::PathBuf;
use std::sync::Arc;

/// The agent whose limits to read and how its account reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageAgent {
    pub id: String,
    pub account: AccountUsage,
}

impl UsageAgent {
    /// An agent without a profile reports limits with usage updates only.
    pub fn new(agent: &crate::agents::AvailableAgent) -> Self {
        Self {
            id: agent.id().to_owned(),
            account: agent
                .profile()
                .map_or(AccountUsage::None, |profile| profile.usage().account()),
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

/// One account source per way of reporting limits; agents whose way has no
/// source report nothing.
#[derive(Default)]
pub struct UsageSources(Vec<(AccountUsage, Arc<dyn AccountUsageSource>)>);

impl UsageSources {
    /// Reads `account` agents' limits from `source`, replacing an earlier one.
    #[must_use]
    pub fn with(mut self, account: AccountUsage, source: Arc<dyn AccountUsageSource>) -> Self {
        self.0.retain(|(kind, _)| *kind != account);
        self.0.push((account, source));
        self
    }
}

impl AccountUsageSource for UsageSources {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>> {
        match self.0.iter().find(|(kind, _)| *kind == agent.account) {
            Some((_, source)) => source.read(agent),
            None => Box::pin(async { Ok(None) }),
        }
    }
}
