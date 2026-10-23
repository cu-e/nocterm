//! Builtin host-backed workspace preparation, environment management and activation.
//!
//! Owns: bounded captured-host commands, exact resource preparation, owned rollback and lifecycle actions.
//! Owns: observed OS-aware managed language installation and workspace-only activation.
//! Does not own: provider registration, credentials, catalog persistence, container filesystems, GUI or AI launch.
//! Ports used: HostExec, Transport, CreatedWorkspaceEnvironment; provided: ProjectPreparation, ImageSearch.
//! Ports provided: WorkspaceEnvironment, WorkspaceContainerActions, WorkspaceSessionFactory, EnvironmentCommandFactory.
//! Settings section: none. Globals: none.
//! Invariants: automatic rollback verifies original ID, project and fresh attempt labels before deletion.
//! Invariants: uncertain creates reconcile on their original executor or retain a recoverable typed attempt.
//! Invariants: all bounded command contexts retain the supplied executor and original cancellation/budget.
//! Invariants: installers observe the actual Linux account/platform and preserve private managed versions.
//! Invariants: only fresh owned Arch containers may initialize absent package databases with an upgrade.
//! Invariants: explicit existing-environment failures never delete resources; activation is workspace-only.
//! Invariants: lifecycle actions stop/remove the original full ID without force, volumes, images or host cleanup.
mod activation;
mod command;
mod environment;
mod image_search;
mod inspect;
mod languages;
mod ownership;
mod prepare;
mod pull_progress;
mod removal;

pub use activation::NativeWorkspaceSessions;
pub use command::NativeEnvironmentCommands;
pub use environment::NativeWorkspaceEnvironment;
pub use image_search::NativeImageSearch;
pub use prepare::NativeProjectPreparation;
pub use removal::NativeWorkspaceContainerActions;

#[cfg(test)]
mod tests;
