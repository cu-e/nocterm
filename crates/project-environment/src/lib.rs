//! Observed workspace tool installation and managed session activation.
//!
//! Owns: actual platform/account probing, managed installers and workspace-only activation.
//! Does not own: resource preparation/removal, rollback, credentials, catalog or ordinary connection policy.
//! Ports used: EnvironmentCommands, EnvironmentCommandFactory, HostExec, Transport.
//! Ports provided: WorkspaceEnvironment, CreatedWorkspaceEnvironment, WorkspaceSessionFactory, WorkspaceCommandActivation.
//! Settings section: none. Globals: none.
//! Invariants: existing environments never receive fresh-owned creation privileges.
//! Invariants: fresh installation requires the actual owned receipt and original supplied command context.
//! Invariants: checked selected versions travel as literal arguments; installation preserves private managed paths.
//! Invariants: actual platform/account facts select installation policy; image references never substitute for observation.
//! Invariants: activation shares installer layout and never sources a mutable environment fragment.
mod activation;
mod environment;
mod languages;

pub use activation::NativeWorkspaceSessions;
pub use environment::NativeWorkspaceEnvironment;
