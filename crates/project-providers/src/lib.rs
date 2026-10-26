//! Typed builtin workspace locations and host-backed preparation roles.
//!
//! Owns: container codecs, checked versions, captured environment facts and lifecycle authority.
//! Owns: pure exact-container requests, command/created-install seams and default workspace directory policy.
//! Does not own: I/O, credentials, persistence, sessions, GPUI, views or provider registration.
//! Ports used: captured HostExec and Transport.
//! Ports provided: ProjectPreparation, ImageSearch, WorkspaceEnvironment, WorkspaceContainerActions.
//! Ports provided: WorkspaceSessionFactory, ContainerSessionFactory, EnvironmentCommands, EnvironmentCommandFactory.
//! Ports provided: CreatedWorkspaceEnvironment, WorkspaceCommandActivation.
//! Settings section: `[workspaces]`. Globals: none.
//! Invariants: container identity includes original host, explicit engine and full ID.
//! Invariants: bounded nonsecret resource locators preserve catalog identity through setup.
//! Invariants: ephemeral owned receipts retain a fresh creation-attempt token.
//! Invariants: host/scoped execution authority is never interchangeable.
//! Invariants: installation grants no catalog/deletion authority; created install requires an owned receipt.
//! Invariants: scoped command contexts retain original budgets; Stop/Delete uses separate captured-host authority.
mod activation;
mod created_environment;
mod environment;
mod environment_commands;
mod error;
mod image_search;
mod language;
mod location;
mod progress;
mod removal;
mod settings;
mod setup;

pub use activation::{WorkspaceCommandActivation, WorkspaceSessionFactory};
pub use created_environment::CreatedWorkspaceEnvironment;
pub use environment::*;
pub use environment_commands::{EnvironmentCommandFactory, EnvironmentCommands};
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
