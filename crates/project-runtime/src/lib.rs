//! Builtin project resource preparation, bounded commands and lifecycle actions.
//!
//! Owns: captured-host command budgets, preparation, owned rollback, bounded search/progress and lifecycle actions.
//! Does not own: environment installation/activation, container filesystems, credentials, catalog or GUI.
//! Ports used: HostExec, CreatedWorkspaceEnvironment.
//! Ports provided: ProjectPreparation, ImageSearch, WorkspaceContainerActions, EnvironmentCommandFactory.
//! Settings section: none. Globals: none.
//! Invariants: command scopes retain the original executor, started budget, cancellation and progress authority.
//! Invariants: creation delegates installation with its actual owned receipt and original command context.
//! Invariants: automatic rollback verifies full container ID, project and fresh attempt labels before deletion.
//! Invariants: uncertain creates reconcile on their original executor or retain a recoverable typed attempt.
//! Invariants: lifecycle actions stop/remove the original full ID without force, volumes, images or host cleanup.
//! Invariants: only bounded recovery creates its deliberate fresh uncancelled command context.
mod command;
mod image_search;
mod inspect;
mod ownership;
mod prepare;
mod pull_progress;
mod removal;

pub use command::NativeEnvironmentCommands;
pub use image_search::NativeImageSearch;
pub use prepare::NativeProjectPreparation;
pub use removal::NativeWorkspaceContainerActions;

#[cfg(test)]
mod tests;
