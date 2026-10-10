use std::{error::Error, sync::Arc};

/// A rejected edit, failed write or interrupted completion.
#[derive(Clone, Debug, thiserror::Error)]
pub enum StoreError {
    #[error("persistence is closing")]
    Closing,
    #[error("the persistence queue is full")]
    Full,
    #[error("latest-desired admission requires optimistic publication")]
    OptimisticRequired,
    #[error("persistence completion was dropped")]
    CompletionDropped,
    #[error("the shared shutdown deadline expired")]
    DeadlineExceeded,
    #[error("an ordered edit was rejected: {0}")]
    Edit(#[source] Arc<dyn Error + Send + Sync>),
    #[error("immediate publication requires an idle durable worker")]
    PendingWrites,
    #[error("multiple persistence participants failed: {first}; {second}")]
    Joined {
        #[source]
        first: Arc<StoreError>,
        second: Arc<StoreError>,
    },
    #[error("snapshot persistence failed: {0}")]
    Backend(#[source] Arc<dyn Error + Send + Sync>),
}

impl StoreError {
    pub fn edit(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Edit(Arc::new(error))
    }

    /// Preserve the backend's typed error and source chain.
    pub fn backend(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Backend(Arc::new(error))
    }
}
