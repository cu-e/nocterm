use nocterm_core::SharedError;
use nocterm_snippets::LibraryError;
use nocterm_store::StoreError;
use std::sync::Arc;
#[derive(Debug, thiserror::Error)]
pub enum SnippetsLoadError {
    #[error(transparent)]
    File(#[from] nocterm_snippets::SnippetStorageError),
    #[error(transparent)]
    Library(#[from] LibraryError),
}

#[derive(Debug, thiserror::Error)]
pub enum SnippetsError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error("Snippet library is read-only until its file is fixed: {0}")]
    ReadOnly(#[source] SharedError<SnippetsLoadError>),
    #[error("Snippet service is shutting down.")]
    Closing,
    #[error("Snippet save queue is full. Wait for pending saves and retry.")]
    Full,
    #[error("Snippet save was interrupted.")]
    Cancelled,
    #[error("Could not persist the snippet library: {0}")]
    Persistence(#[source] StoreError),
}

impl SnippetsError {
    pub fn read_only(source: Arc<SnippetsLoadError>) -> Self {
        Self::ReadOnly(SharedError::from_arc(source))
    }
}

impl From<StoreError> for SnippetsError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Closing => Self::Closing,
            StoreError::Full => Self::Full,
            StoreError::CompletionDropped => Self::Cancelled,
            StoreError::Edit(ref source) => source
                .downcast_ref::<LibraryError>()
                .cloned()
                .map_or(Self::Persistence(error), Self::Library),
            error => Self::Persistence(error),
        }
    }
}
