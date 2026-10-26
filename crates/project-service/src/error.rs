use std::sync::Arc;

use nocterm_core::SharedError;
use nocterm_project_model::{CatalogError, ProjectId, ProjectMetadataError, ProjectStorageError};
use nocterm_store::StoreError;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProjectsError {
    #[error("workspace capability installation failed: {0}")]
    Installation(#[source] nocterm_core::SharedCause),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Metadata(#[from] ProjectMetadataError),
    #[error("the project catalog is read-only until its original load error is fixed: {0}")]
    ReadOnly(#[source] SharedError<ProjectStorageError>),
    #[error("the original project definition binding is stale: {0}")]
    StaleBinding(ProjectId),
    #[error("the original project window claim is stale: {0}")]
    StaleClaim(ProjectId),
    #[error("the project claim generation is exhausted")]
    RevisionExhausted,
    #[error("the project installation is shutting down")]
    Closing,
    #[error("the project publication queue is full")]
    Full,
    #[error("the project publication completion was interrupted")]
    Interrupted,
    #[error("project catalog publication failed: {0}")]
    Persistence(#[source] StoreError),
}

impl ProjectsError {
    pub fn installation(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Installation(nocterm_core::SharedCause::new(source))
    }
    pub(crate) fn read_only(error: Arc<ProjectStorageError>) -> Self {
        Self::ReadOnly(SharedError::from_arc(error))
    }
}

impl From<StoreError> for ProjectsError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Closing => Self::Closing,
            StoreError::Full => Self::Full,
            StoreError::CompletionDropped => Self::Interrupted,
            StoreError::Edit(ref source) => source
                .downcast_ref::<CatalogError>()
                .cloned()
                .map_or(Self::Persistence(error), Self::Catalog),
            error => Self::Persistence(error),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum WorkspaceRemovalError {
    #[error(transparent)]
    Action(#[from] nocterm_project_providers::ContainerActionError),
    #[error("workspace settings could not be removed: {0}")]
    Catalog(#[from] ProjectsError),
}
