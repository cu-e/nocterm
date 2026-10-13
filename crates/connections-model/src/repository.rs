//! Blocking document ports. Load and save calls must run off the foreground executor.
use crate::{facts::Known, profiles::Profiles, recents::Recents};
use std::{error::Error, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionDocument {
    Profiles,
    Recents,
    Facts,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageOperation {
    Read,
    Write,
}
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct ConnectionStorageError {
    document: ConnectionDocument,
    operation: StorageOperation,
    #[source]
    source: Box<dyn Error + Send + Sync>,
}
impl ConnectionStorageError {
    pub fn new(
        document: ConnectionDocument,
        operation: StorageOperation,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            document,
            operation,
            source: Box::new(source),
        }
    }
    pub fn document(&self) -> ConnectionDocument {
        self.document
    }
    pub fn operation(&self) -> StorageOperation {
        self.operation
    }
}
#[derive(Clone, Debug, Default)]
pub struct LoadedConnections {
    pub profiles: Profiles,
    pub recents: Recents,
    pub known: Known,
    pub read_only: Option<Arc<ConnectionStorageError>>,
}
/// Native acquisition implements per-document fallback policy and retains malformed-profile diagnostics.
pub trait ConnectionRepository: Send + Sync + 'static {
    fn load(&self) -> LoadedConnections;
    fn save_profiles(&self, profiles: &Profiles) -> Result<(), ConnectionStorageError>;
    fn save_recents(&self, recents: &Recents) -> Result<(), ConnectionStorageError>;
    fn save_facts(&self, known: &Known) -> Result<(), ConnectionStorageError>;
}
