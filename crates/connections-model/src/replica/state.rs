//! The last state both sides agreed on, which tells a local change from a remote one.
use super::{ReplicaCursor, ReplicaScope, ReplicaVersion};
use crate::profiles::{Profile, ProfileId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A profile as it was when the local copy and the replica last agreed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncedEntry {
    pub scope: ReplicaScope,
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
    /// Where the user wants a profile to live when that differs from where it was
    /// agreed to live (or it has not been sent yet). Cleared once the profile is sent.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    placements: HashMap<ProfileId, ReplicaScope>,
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

    /// The scope the user asked the profile to be moved or created in, until it is sent.
    pub fn placement(&self, id: ProfileId) -> Option<&ReplicaScope> {
        self.placements.get(&id)
    }

    pub fn placements(&self) -> impl Iterator<Item = (ProfileId, &ReplicaScope)> {
        self.placements.iter().map(|(id, scope)| (*id, scope))
    }

    /// Asks for the profile to live in `scope` from the next synchronization on.
    pub fn place(&mut self, id: ProfileId, scope: ReplicaScope) {
        self.placements.insert(id, scope);
    }

    /// The request was carried out, refused, or no longer matters.
    pub fn unplace(&mut self, id: ProfileId) {
        self.placements.remove(&id);
    }

    pub fn advance(&mut self, cursor: ReplicaCursor) {
        self.cursor = Some(cursor);
    }

    /// Records agreement on `entry`; the replica's version is readable again.
    pub fn agree(&mut self, id: ProfileId, entry: SyncedEntry) {
        self.unrecognized.remove(&id);
        self.placements.remove(&id);
        self.entries.insert(id, entry);
    }

    /// Both sides no longer hold the profile in the scope it was agreed in. A move
    /// carries its new scope in the action that follows, not in the state.
    pub fn forget(&mut self, id: ProfileId) {
        self.unrecognized.remove(&id);
        self.placements.remove(&id);
        self.entries.remove(&id);
    }

    pub fn mark_unrecognized(&mut self, id: ProfileId) {
        self.unrecognized.insert(id);
    }
}
