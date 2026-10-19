//! Blocking library storage and draft identity capabilities supplied by composition.
use crate::Library;
use std::error::Error;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageOperation {
    Read,
    Write,
}
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct SnippetStorageError {
    operation: StorageOperation,
    #[source]
    source: Box<dyn Error + Send + Sync>,
}
impl SnippetStorageError {
    pub fn new(operation: StorageOperation, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            operation,
            source: Box::new(source),
        }
    }
    pub fn operation(&self) -> StorageOperation {
        self.operation
    }
}
/// Blocking contract; the caller chooses acquisition/execution phase.
pub trait SnippetRepository: Send + Sync + 'static {
    fn load(&self) -> Result<Option<Library>, SnippetStorageError>;
    fn save(&self, library: &Library) -> Result<(), SnippetStorageError>;
}
/// Called once when an actual new editor draft is constructed.
pub trait SnippetIds: Send + Sync + 'static {
    fn next(&self) -> uuid::Uuid;
}
