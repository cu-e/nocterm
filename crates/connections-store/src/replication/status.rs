//! What the replication reports to its observers.
use crate::ConnectionsError;
use nocterm_connections_model::{
    profiles::{Profile, ProfileId},
    replica::{ReplicaError, ReplicaScope},
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
    /// The replica refused to change `profile` in `scope`, because the account may not
    /// write there. When `restored`, the local profile is back to the last agreed
    /// version; otherwise it stays local and is sent to the personal scope.
    Refused {
        profile: Box<Profile>,
        scope: ReplicaScope,
        restored: bool,
    },
}
