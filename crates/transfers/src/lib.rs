//! Bounded transfer requests, collision policy and progress transitions.
//!
//! Owns: admission/discovery/stream limits, pure progress accounting and transfer ports.
//! Does not own: scheduling, clocks, random IDs or filesystem/network I/O.
//! Ports used: `RemoteFs`. Ports provided: `TransferService`, `TransferRequests`.
//! Settings section: none. Globals: none.
//! Invariants: requests are bounded before admission; acknowledged bytes advance
//! progress; completion requires discovery and workers to finish; an `OwnedTransfers`
//! book only addresses ids it submitted and keeps final snapshots the runtime may trim.
//! Runtime diagnostics retain the original concrete cause through core shared error ownership.
mod collision;
mod endpoint;
mod limits;
mod owned;
mod progress;
mod request;
mod service;
pub use collision::{
    CollisionDecision, CollisionPolicy, collision_decision, renamed, valid_download_name,
    valid_remote_source,
};
pub use endpoint::TransferEndpoint;
pub use limits::{
    DiscoveryBudget, DiscoveryLimits, LimitKind, QueueLimits, StreamBudget, StreamLimits,
    TransferLimits,
};
pub use owned::{OwnedLimits, OwnedTransferError, OwnedTransfers};
pub use progress::{
    Progress, ProgressTracker, TransferCause, TransferDirection, TransferEvent, TransferIssue,
    TransferOperation, TransferPath, TransferState,
};
pub use request::{DownloadRequest, RelayRequest, Request, UploadRequest, valid_relay_source};
pub use service::{TransferError, TransferId, TransferRequests, TransferService};

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
