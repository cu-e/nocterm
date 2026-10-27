//! The port a remote copy of the saved profiles implements, and the reconciliation rules.
//!
//! The local document stays the source the application reads. A replica is a second
//! copy, kept by another party (a hub), that changes independently. The rules in
//! [`rules`] decide, per profile, what to do when the local copy, the replica and the
//! last agreed state differ; they never perform I/O.
mod state;

pub mod rules;

pub use state::{ReplicaState, SyncedEntry};

use crate::profiles::{Profile, ProfileId};
use futures::{future::BoxFuture, stream::BoxStream};
use nocterm_core::SharedCause;
use serde::{Deserialize, Serialize};

/// Where a replicated profile lives: the user's own space or an organization's.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReplicaScope {
    Personal,
    Organization(String),
}

/// How the account may use a space of the replica.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaAccess {
    /// Profiles of the space are shown and used; the replica refuses changes to them.
    Read,
    /// Profiles of the space may be created, changed, moved out of it and deleted.
    Write,
}

/// A place the replica keeps profiles in, as the account can use it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaSpace {
    pub scope: ReplicaScope,
    /// What the user calls it: "Personal", or the name of the organization.
    pub name: String,
    pub access: ReplicaAccess,
}

/// An opaque position in the replica's change feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplicaCursor(String);

impl ReplicaCursor {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The replica's revision of one profile, used as the expectation of the next write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplicaVersion(u64);

impl ReplicaVersion {
    pub fn new(version: u64) -> Self {
        Self(version)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// What the replica holds for a profile.
#[derive(Clone, Debug, PartialEq)]
pub enum RemoteBody {
    Present(Box<Profile>),
    /// A tombstone: the profile was deleted and the deletion is itself a version.
    Deleted,
    /// An object this client cannot decode, usually written by a newer client.
    Unrecognized,
}

/// One entry of the change feed.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteProfile {
    pub scope: ReplicaScope,
    pub id: ProfileId,
    pub version: ReplicaVersion,
    pub body: RemoteBody,
}

/// Changes after a cursor, in the order the replica made them.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplicaPage {
    pub changes: Vec<RemoteProfile>,
    /// Where to continue reading after these changes.
    pub cursor: ReplicaCursor,
    /// More changes are available immediately after `cursor`.
    pub more: bool,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReplicaError {
    /// The replica cannot be reached now; local work continues.
    #[error("The profile replica is not reachable.")]
    Unavailable,
    /// The expected version is no longer the replica's current one.
    #[error("Profile {id} changed on the replica.")]
    Conflict { id: ProfileId },
    /// The caller may not change this profile in its scope.
    #[error("Profile {id} cannot be changed in its scope.")]
    Forbidden { id: ProfileId },
    #[error("The profile replica failed: {0}")]
    Other(#[source] SharedCause),
}

/// A remote copy of the saved profiles, one sealed object per profile.
pub trait ProfileReplica: Send + Sync {
    /// Changes after `after`; `None` reads from the beginning.
    fn changes(
        &self,
        after: Option<ReplicaCursor>,
    ) -> BoxFuture<'static, Result<ReplicaPage, ReplicaError>>;

    /// Writes `profile`. `expected` is the version this client last saw, or `None`
    /// to create; a replica holding another version answers `Conflict`.
    fn put(
        &self,
        scope: ReplicaScope,
        profile: Profile,
        expected: Option<ReplicaVersion>,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>>;

    /// Replaces the profile with a tombstone under the same expectation rules.
    fn delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    ) -> BoxFuture<'static, Result<ReplicaVersion, ReplicaError>>;

    /// Yields once whenever the replica may have new changes.
    fn watch(&self) -> BoxStream<'static, ()>;

    /// The spaces the replica serves and what the account may do in each, as last learned.
    /// A replica with a single, writable space of its own reports none.
    fn spaces(&self) -> Vec<ReplicaSpace> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
