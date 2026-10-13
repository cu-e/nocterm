use nocterm_connections_model::profiles::ProfileError;
use nocterm_connections_model::repository::ConnectionStorageError;
use nocterm_store::StoreError;
use std::sync::Arc;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ConnectionsError {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error("Saved connections are read-only until the file is fixed: {0}")]
    ReadOnly(#[source] Arc<ConnectionStorageError>),
    #[error("Connections service is shutting down.")]
    Closing,
    #[error("Connection save queue is full. Wait for pending saves to finish and retry.")]
    Full,
    #[error("Connections service closed before saving.")]
    Cancelled,
    #[error("Could not save connections: {0}")]
    Persistence(#[source] StoreError),
}

impl From<StoreError> for ConnectionsError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Edit(source) => source
                .downcast_ref::<Self>()
                .cloned()
                .unwrap_or_else(|| Self::Persistence(StoreError::Edit(source))),
            StoreError::Closing => Self::Closing,
            StoreError::Full => Self::Full,
            StoreError::CompletionDropped => Self::Cancelled,
            error => Self::Persistence(error),
        }
    }
}
