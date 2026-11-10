//! What the replication reports to its observers.
use crate::ConnectionsError;
use nocterm_connections_model::{
    profiles::{Profile, ProfileId},
    replica::ReplicaError,
};
use nocterm_store::StoreError;

/// Where synchronization stands.
#[derive(Clone, Debug)]
pub enum ReplicationStatus {
    /// No replica is attached; profiles stay local.
    Detached,
    Syncing,
    /// Local and remote profiles agree.
    Synced,
    /// The replica could not be reached; local work continues and sync retries on the
    /// next local change or remote notification.
    Offline,
    /// The keys that open the replica are closed; the server may be reachable. Local work
    /// continues and sync resumes when the keys open and the replica notifies, or on the
    /// next attach.
    Locked,
    /// The replica holds a profile this client cannot read; the local copy is untouched.
    NeedsNewerClient,
    Failed(ReplicationFailure),
}

/// Why a synchronization could not finish, or why one profile was skipped.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ReplicationFailure {
    /// Saved connections failed to load, so an empty local copy would look like a
    /// deletion of everything.
    #[error("Saved connections could not be read, so nothing was synchronized.")]
    LocalUnavailable,
    /// A profile kept changing under the synchronization.
    #[error("Profiles kept changing during synchronization.")]
    Conflicts,
    #[error(transparent)]
    Feed(#[from] super::ReplicationFeedError),
    #[error(transparent)]
    Replica(#[from] ReplicaError),
    /// The local document refused one profile; the others were synchronized.
    #[error("Connection {id} could not be saved locally: {source}")]
    Rejected {
        id: ProfileId,
        #[source]
        source: ConnectionsError,
    },
    #[error("Connections could not be updated: {0}")]
    Local(#[source] ConnectionsError),
    #[error("Synchronization state could not be kept: {0}")]
    State(#[source] StoreError),
}

#[derive(Clone, Debug)]
pub enum ReplicationEvent {
    /// A remote version replaced this unsynchronized local edit.
    Overwritten {
        id: ProfileId,
        discarded: Box<Profile>,
    },
    Failed(ReplicationFailure),
    /// The replica refused to change `profile`, because the account may not. When
    /// `restored`, the local profile is back to the last agreed version; otherwise it stays
    /// local only and is not sent again until it changes.
    Refused {
        profile: Box<Profile>,
        restored: bool,
    },
}
