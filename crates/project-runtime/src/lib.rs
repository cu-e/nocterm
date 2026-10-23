//! Builtin host-backed workspace preparation and container session adaptation.
//!
//! Owns: bounded captured-host command execution, exact engine/container routing,
//! observed OS-aware managed language installation, bounded search/progress, owned rollback
//! scoped container filesystem streaming/publication and explicit lifecycle actions.
//! Does not own: provider registration, credentials, catalog persistence, GUI or AI launch.
//! Ports used: HostExec, Transport. Ports provided: ProjectPreparation,
//! ImageSearch, WorkspaceEnvironment, WorkspaceContainerActions, WorkspaceSessionFactory,
//! ContainerSessionFactory and RemoteFs.
//! Settings section: none. Globals: none.
//! Invariants: automatic rollback never deletes attached/direct resources; it verifies ID and
//! project and fresh attempt labels; uncertain creates reconcile on their original
//! executor or retain a recoverable typed attempt; container sessions expose scoped
//! execution and a bounded container-only filesystem; host filesystem never leaks.
//! Search never pulls; all commands retain the supplied
//! executor. Shared installers observe the actual Linux account/platform and preserve
//! private managed versions; explicit existing-environment failures never delete resources.
//! Only fresh owned Arch containers may initialize absent package databases with an upgrade;
//! existing environments require administrator preparation. Activation is workspace-only.
//! Explicit lifecycle actions inspect the original full ID; deletion stops then removes
//! that container without force, volumes, images, names or host-directory cleanup.
mod activation;
mod command;
mod environment;
mod filesystem;
mod image_search;
mod inspect;
mod languages;
mod ownership;
mod prepare;
mod pull_progress;
mod removal;
mod session;

pub use activation::NativeWorkspaceSessions;
pub use command::NativeEnvironmentCommands;
pub use environment::NativeWorkspaceEnvironment;
pub use image_search::NativeImageSearch;
pub use prepare::NativeProjectPreparation;
pub use removal::NativeWorkspaceContainerActions;
pub use session::NativeContainerSessions;

#[cfg(test)]
mod tests;
