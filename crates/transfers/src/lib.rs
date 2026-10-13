//! Bounded transfer requests, collision policy and progress transitions.
//!
//! Owns: admission/discovery/stream limits, pure progress accounting and transfer ports.
//! Does not own: scheduling, clocks, random IDs or filesystem/network I/O.
//! Ports used: `RemoteFs`. Ports provided: `TransferService`.
//! Settings section: none. Globals: none.
//! Invariants: requests are bounded before admission; acknowledged bytes advance
//! progress; completion requires discovery and workers to finish.
//! Runtime diagnostics retain the original concrete cause through core shared error ownership.
mod collision;
mod limits;
mod progress;
mod request;
mod service;
pub use collision::{
    CollisionDecision, CollisionPolicy, collision_decision, renamed, valid_download_name,
    valid_remote_source,
};
pub use limits::{
    DiscoveryBudget, DiscoveryLimits, LimitKind, QueueLimits, StreamBudget, StreamLimits,
    TransferLimits,
};
pub use progress::{
    Progress, ProgressTracker, TransferCause, TransferDirection, TransferEvent, TransferIssue,
    TransferOperation, TransferPath, TransferState,
};
pub use request::{DownloadRequest, Request, UploadRequest};
pub use service::{TransferError, TransferId, TransferService};

#[cfg(feature = "test-support")]
pub mod testing;
