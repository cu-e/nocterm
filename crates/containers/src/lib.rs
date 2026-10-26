//! Containers on a host, independent of any UI.
//!
//! Owns: engine detection, CLI requests, listing parsing and container actions.
//! Does not own: session creation, daemon sockets, credential storage or views.
//! Ports used: HostExec. Ports provided: none. Settings section: none. Globals: none.
//! Invariants: operations use the supplied host executor and its authority;
//! unavailable engines remain distinct from typed execution or decoding failures;
//! listing output is bounded.
//!
//! A host is asked about its containers through a [`HostExec`], with the
//! container engine's own command line: Docker's, or Podman's where Docker
//! is not installed. Nothing here opens a connection or talks to a daemon
//! socket, so a remote host is managed over the session already open to it,
//! with the rights of the user signed in.
//!
//! [`Engine`] builds the commands — listing, the event stream that says
//! when to list again, actions, and the programs a tab runs (a container's
//! log, a shell inside it) — and [`Snapshot`] is what a listing found,
//! grouped by Compose project for display.
//!
//! [`HostExec`]: nocterm_session::HostExec

mod engine;
mod error;
mod model;
mod parse;

pub use engine::{Action, Engine, detect};
pub use error::{ContainersError, ListingEntity, ListingError};
pub use model::{Container, Group, Image, Snapshot, State};

/// Decode a bounded container listing acquired by a host adapter.
pub fn decode_containers(output: &str) -> Result<Vec<Container>, ListingError> {
    parse::containers(output)
}

/// Decode a bounded image listing acquired by a host adapter.
pub fn decode_images(output: &str) -> Result<Vec<Image>, ListingError> {
    parse::images(output)
}
