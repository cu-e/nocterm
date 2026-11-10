//! Blocking catalog storage supplied explicitly by application composition.

use crate::{Catalog, ProjectStorageError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectRepositoryOperation {
    Load,
    Save,
}

/// Call on the existing background store executor, never the GUI foreground.
///
/// A missing file is None, distinct from an empty or invalid catalog. Save
/// returns success only after publication commits. Serialization, shared writer
/// ordering and checked service revisions do not create a second queue here.
pub trait ProjectRepository: Send + Sync + 'static {
    fn load(&self) -> Result<Option<Catalog>, ProjectStorageError>;
    fn save(&self, catalog: &Catalog) -> Result<(), ProjectStorageError>;
}
