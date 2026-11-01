//! Streaming upload, download and remote-to-remote relay execution over explicit native and remote ports.
//!
//! Owns: bounded scheduling, entropy, cancellation, reservations and cleanup.
//! Does not own: native filesystem syscalls, UI, request rules or progress rules.
//! Ports used: optional `LocalFs` (absent in remote-only runtimes), `LocalDirectory`, `LocalSource`, `LocalStage`, `RemoteFs`.
//! Ports provided: `TransferService`, `TransferRequests`. Settings section: none. Globals: none.
//! Invariants: only acknowledged bytes count; held capabilities authorize output;
//! cancellation drains work before terminal progress and retained-runtime release.
mod batch;
mod blocking;
mod chunks;
mod discovery;
mod download;
mod relay;
mod relay_discovery;
mod remote_destination;
mod reservations;
mod runtime;
mod scheduler;
mod upload;
pub use runtime::{TransferRuntime, TransferRuntimeDeps, TransferRuntimeError};
