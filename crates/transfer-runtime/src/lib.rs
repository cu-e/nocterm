//! Streaming upload/download execution over explicit native and remote ports.
//!
//! Owns: bounded scheduling, entropy, cancellation, reservations and cleanup.
//! Does not own: native filesystem syscalls, UI, request rules or progress rules.
//! Ports used: `LocalFs`, `LocalDirectory`, `LocalSource`, `LocalStage`, `RemoteFs`.
//! Ports provided: `TransferService`. Settings section: none. Globals: none.
//! Invariants: only acknowledged bytes count; held capabilities authorize output;
//! cancellation drains work before terminal progress and retained-runtime release.
mod batch;
mod blocking;
mod discovery;
mod download;
mod reservations;
mod runtime;
mod scheduler;
mod upload;
pub use runtime::{TransferRuntime, TransferRuntimeDeps, TransferRuntimeError};
