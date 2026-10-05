//! Containers on a host, independent of any UI.
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
mod model;
mod parse;

pub use engine::{Action, ContainersError, Engine, detect};
pub use model::{Container, Group, Image, Snapshot, State};
