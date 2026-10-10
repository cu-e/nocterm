//! Shared filesystem primitives and standard application locations.
//!
//! Owns: paths, exclusive private creation, atomic publication, TOML persistence,
//! ordered write queues and shutdown deadlines.
//! Does not own: transport protocols, encryption, storage schemas, views or tasks.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: staging is exclusive; failed precommit writes retain old bytes;
//! private publication replaces final symlinks, while TOML saves follow them.
//! Parent directories must be trusted; traversal capabilities belong to adapters.

pub mod fs;
pub mod paths;
pub mod persist;

pub use paths::{Paths, PathsError};
pub use persist::PersistError;
