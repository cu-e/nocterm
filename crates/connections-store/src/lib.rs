//! Ordered saved-connection publication and committed profile authority.
//! Owns: checked profile admission, profile/recent persistence composition,
//! immutable committed authority and coupled capture.
//! Does not own: files/native defaults/IDs, transports, settings/views/editor state,
//! DNS/OS/country/flags or observation jobs.
//! Ports used: SnapshotWriter, ConnectionClock, ShutdownCoordinator.
//! Ports provided: ConnectionStore, ProfilePublications. Settings: none. Globals: none.
//! Invariants: physical profile success precedes receipts; derived recents precede
//! successful receipts; independent recents survive profile failure; capture uses
//! the shared 180-ms shutdown driver, without another worker or deadline.
mod connections;
mod error;
mod prepared;
mod publication;
pub use connections::ConnectionStore;
pub use error::ConnectionsError;
pub use nocterm_connections_model::profile_authority::{
    ProfileAuthorityRow, ProfilePublication, ProfileStamp,
};
pub use publication::projection::ProfilePublications;
