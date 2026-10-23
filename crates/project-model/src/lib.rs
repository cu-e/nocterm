//! Provider-neutral workspace definitions and stable history context identity.
//!
//! Owns: supplied stable IDs, literal absolute POSIX folders, saved parent and
//! ordered jump routes, bounded opaque provider locators, provenance, checked
//! catalog edits and source generations.
//! Does not own: sessions, filesystem/process I/O, clocks, entropy, GPUI or views.
//! Ports used: session target values. Ports provided: blocking ProjectRepository
//! and supplied ProjectMetadataSource.
//! Shared deterministic port fakes require test-support.
//! Settings section: none. Globals: none.
//! Invariants: IDs survive folder/name edits; remote paths use POSIX semantics;
//! malformed catalogs are errors; unknown providers retain identity and locators;
//! legacy policy metadata is read and discarded.

mod catalog;
mod catalog_state;
mod definition;
mod error;
mod identity;
mod metadata;
mod repository;
mod root;
mod target;

#[cfg(feature = "test-support")]
pub mod testing;

pub use catalog::{Catalog, MAX_CATALOG_BYTES, MAX_PROJECTS, ProjectGroups};
pub use catalog_state::{CatalogState, ProjectStamp};
pub use definition::{
    HostLocator, ParentConnectionKey, ProjectDefinition, ProjectDraft, ProjectSource,
};
pub use error::{CatalogError, ProjectMetadataError, ProjectStorageError, ProjectValidationError};
pub use identity::{ContextId, ProjectId, ProjectIdParseError};
pub use metadata::{ProjectIdentity, ProjectMetadataSource};
pub use repository::{ProjectRepository, ProjectRepositoryOperation};
pub use root::RootPath;
pub use target::{MAX_RESOURCE_BYTES, ProjectTarget, ProviderId, ProviderResourceRef};
