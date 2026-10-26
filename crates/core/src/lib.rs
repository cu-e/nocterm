//! Shared error ownership, filesystem primitives and standard application locations.
//!
//! Owns: paths, exclusive private creation, atomic publication, bounded no-follow reads of
//! regular files, TOML persistence,
//! ordered write queues, shutdown deadlines, immutable concrete error causes and the wall-clock
//! contract (`clock::{WallClock, WallTime}`; no I/O, no time-zone data).
//! Does not own: transport protocols, encryption, storage schemas, views or tasks.
//! Ports used: none. Ports provided: `clock::WallClock`, implemented by `nocterm-local`; its
//! test fake is behind `test-support`. Settings section: none. Globals: none.
//! Invariants: staging is exclusive; failed precommit writes retain old bytes;
//! private publication replaces final symlinks, while TOML saves follow them; a bounded read
//! opens before it checks, refuses links, pipes and devices, and never exceeds its cap.
//! Parent directories must be trusted; traversal capabilities belong to adapters.

pub mod clock;
pub mod error;
pub mod fs;
pub mod paths;
pub mod persist;

pub use error::{SharedCause, SharedError};
pub use paths::{Paths, PathsError};
pub use persist::PersistError;
