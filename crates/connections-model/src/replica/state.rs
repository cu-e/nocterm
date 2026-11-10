//! The last state both sides agreed on, which tells a local change from a remote one.
use super::{ReplicaCursor, ReplicaVersion};
use crate::profiles::{Profile, ProfileId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A profile as it was when the local copy and the replica last agreed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncedEntry {
    pub version: ReplicaVersion,
    pub profile: Profile,
}

/// What is persisted between runs: the feed position, the agreed profiles and the
/// profiles the replica holds in a form this client cannot read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplicaState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<ReplicaCursor>,
    #[serde(default)]
    entries: HashMap<ProfileId, SyncedEntry>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    unrecognized: HashSet<ProfileId>,
}

impl ReplicaState {
    pub fn cursor(&self) -> Option<&ReplicaCursor> {
        self.cursor.as_ref()
    }

    pub fn entry(&self, id: ProfileId) -> Option<&SyncedEntry> {
        self.entries.get(&id)
    }

    pub fn entries(&self) -> impl Iterator<Item = (ProfileId, &SyncedEntry)> {
        self.entries.iter().map(|(id, entry)| (*id, entry))
    }

    /// The replica holds a version of this profile that this client cannot read.
    pub fn is_unrecognized(&self, id: ProfileId) -> bool {
        self.unrecognized.contains(&id)
    }

    pub fn has_unrecognized(&self) -> bool {
        !self.unrecognized.is_empty()
    }

    pub fn advance(&mut self, cursor: ReplicaCursor) {
        self.cursor = Some(cursor);
    }

    /// Records agreement on `entry`, whether this client sent it or took the replica's
    /// version; the replica's version is readable again.
    pub fn agree(&mut self, id: ProfileId, entry: SyncedEntry) {
        self.unrecognized.remove(&id);
        self.entries.insert(id, entry);
    }

    /// Neither side holds the profile any more: the agreement goes.
    pub fn forget(&mut self, id: ProfileId) {
        self.unrecognized.remove(&id);
        self.entries.remove(&id);
    }

    pub fn mark_unrecognized(&mut self, id: ProfileId) {
        self.unrecognized.insert(id);
    }
}
