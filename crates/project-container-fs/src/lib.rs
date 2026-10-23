//! Container-scoped sessions and bounded remote filesystem adaptation.
//!
//! Owns: exact container command routing, container-only sessions and filesystem streaming/publication.
//! Does not own: host authentication, preparation/rollback, environment installation or catalog persistence.
//! Ports used: HostExec, Transport, WorkspaceCommandActivation; provided: ContainerSessionFactory, RemoteFs.
//! Settings section: none. Globals: none.
//! Invariants: every command retains the supplied executor, engine and full container ID.
//! Invariants: workspace activation precedes container wrapping; filesystem commands always use root cwd.
//! Invariants: nested sessions share one scoped filesystem and never fall back to the host filesystem.
//! Invariants: bounded streams and staged publication preserve identity checks described in ADR 0076.
mod filesystem;
mod session;

pub use session::NativeContainerSessions;

#[cfg(test)]
mod tests;
