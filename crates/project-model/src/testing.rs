//! Shared deterministic catalog and metadata ports, behind test-support.
mod metadata;
mod repository;

pub use metadata::SequenceProjectMetadata;
pub use repository::{InMemoryProjectRepository, RepositoryPause};
