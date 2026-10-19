//! Shared error ownership, filesystem primitives and standard application locations.
//!
//! Owns: paths, exclusive private creation, atomic publication, TOML persistence,
//! ordered write queues, shutdown deadlines and immutable concrete error causes.
//! Does not own: transport protocols, encryption, storage schemas, views or tasks.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: staging is exclusive; failed precommit writes retain old bytes;
//! private publication replaces final symlinks, while TOML saves follow them.
//! Parent directories must be trusted; traversal capabilities belong to adapters.
//! Byte and typed producer snapshots share one private stage and commit boundary.

pub mod error;
pub mod fs;
pub mod paths;
pub mod persist;
pub mod resident_error;
mod resident_lease;

pub use error::{SharedCause, SharedError};
pub use paths::{Paths, PathsError};
pub use persist::PersistError;
pub use resident_error::SharedResidentError;
pub use resident_lease::SharedResidentLease;
