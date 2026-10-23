//! Typed builtin workspace locations and host-backed preparation roles.
//!
//! Owns: container codecs, checked language versions, captured environment authority and facts,
//! explicit container lifecycle authority and default container workspace directory policy.
//! Does not own: I/O, credentials, persistence, sessions, GPUI, views or provider registration.
//! Ports used: captured HostExec and Transport. Ports provided: ProjectPreparation,
//! ImageSearch, WorkspaceEnvironment, WorkspaceContainerActions, WorkspaceSessionFactory
//! and ContainerSessionFactory.
//! Settings section: `[workspaces]`. Globals: none.
//! Invariants: container identity includes original host, explicit engine and full ID;
//! resource references are bounded nonsecret locators; ephemeral owned receipts
//! retain a fresh creation-attempt token; setup never changes catalog identity.
//! Language selections and observations are ephemeral; explicit host/scoped execution
//! is never interchangeable. Managed installation never grants catalog or deletion authority;
//! container Stop/Delete requires its separate explicit captured-host contract.
mod activation;
mod environment;
mod error;
mod image_search;
mod language;
mod location;
mod progress;
mod removal;
mod settings;
mod setup;

pub use activation::WorkspaceSessionFactory;
pub use environment::*;
pub use error::{ContainerActionError, EnvironmentError, ProjectSetupError};
pub use image_search::*;
pub use language::{LanguageSelection, ProgrammingLanguage, VersionChoice};
pub use location::{CONTAINER_PROVIDER, ContainerLocation, ContainerOrigin};
pub use progress::{SetupUpdate, TransferProgress};
pub use removal::*;
pub use settings::WorkspacesSettings;
pub use setup::*;

#[cfg(test)]
mod tests;

#[cfg(feature = "test-support")]
pub mod testing;
