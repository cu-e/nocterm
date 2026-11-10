//! Three-way reconciliation of one profile: local `L`, remote `R` and the last agreed `S`.
//!
//! The rules are pure. They name what must happen and leave doing it, and the order
//! of side effects, to the caller. A remote change always wins a conflict; the local
//! edit it replaces is reported so the user can be told. A change older than the
//! agreed version is a late entry of the feed, never a newer state, and is ignored.
use super::{RemoteBody, RemoteProfile, ReplicaState, ReplicaVersion, SyncedEntry};
use crate::profiles::{Profile, ProfileId, Profiles};
use std::collections::{HashMap, HashSet};

/// One effect the reconciliation asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplicaAction {
    /// Write `profile` locally if the local copy still is `expected` (`None`: absent),
    /// then record agreement at `version`.
    SaveLocal {
        version: ReplicaVersion,
        profile: Box<Profile>,
        expected: Option<Box<Profile>>,
    },
    /// Delete the local profile if it still is `expected`, then forget it.
    DeleteLocal {
        id: ProfileId,
        expected: Box<Profile>,
    },
    /// Send `profile`; `expected` is the replica's version to replace (`None`: create).
    Push {
        profile: Box<Profile>,
        expected: Option<ReplicaVersion>,
    },
    /// Send a deletion replacing the replica's `expected` version.
    PushDelete {
        id: ProfileId,
        expected: ReplicaVersion,
    },
    /// Both sides already hold `profile`; record agreement at `version`.
    MarkSynced {
        version: ReplicaVersion,
        profile: Box<Profile>,
    },
    /// Neither side holds the profile any more; drop the agreement.
    Forget { id: ProfileId },
    /// A local edit was replaced by the remote version.
    Overwritten {
        id: ProfileId,
        discarded: Box<Profile>,
    },
    /// The remote version cannot be read; leave the local profile alone.
    NeedsNewerClient { id: ProfileId },
}

impl ReplicaAction {
    pub fn id(&self) -> ProfileId {
        match self {
            Self::SaveLocal { profile, .. }
            | Self::Push { profile, .. }
            | Self::MarkSynced { profile, .. } => profile.id,
            Self::DeleteLocal { id, .. }
            | Self::PushDelete { id, .. }
            | Self::Forget { id }
            | Self::Overwritten { id, .. }
            | Self::NeedsNewerClient { id } => *id,
        }
    }
}

/// Reconciles every profile known to any of the three sides.
///
/// `changes` are the remote changes read since the state's cursor; a profile without
/// one, or with only changes older than its agreed version, is unchanged remotely. A
/// profile the replica holds unreadably stays untouched until a readable version of it
/// arrives.
pub fn plan(
    local: &Profiles,
    changes: &[RemoteProfile],
    state: &ReplicaState,
) -> Vec<ReplicaAction> {
    let latest = latest_changes(changes, state);
    let local_by_id: HashMap<_, _> = local.iter().map(|profile| (profile.id, profile)).collect();
    let mut remembered: Vec<ProfileId> = state.entries().map(|(id, _)| id).collect();
    remembered.sort_unstable();
    let mut seen = HashSet::new();
    local
        .iter()
        .map(|profile| profile.id)
        .chain(changes.iter().map(|change| change.id))
        .chain(remembered)
        .filter(|id| seen.insert(*id))
        .filter(|id| !state.is_unrecognized(*id) || latest.contains_key(id))
        .flat_map(|id| {
            reconcile(
                local_by_id.get(&id).copied(),
                latest.get(&id).copied(),
                state.entry(id),
            )
        })
        .collect()
}

/// The change that stands for each profile: the later one of the feed. Late entries older
/// than the agreed version never stand.
fn latest_changes<'a>(
    changes: &'a [RemoteProfile],
    state: &ReplicaState,
) -> HashMap<ProfileId, &'a RemoteProfile> {
    changes
        .iter()
        .filter(|change| !is_stale(change, state.entry(change.id)))
        .map(|change| (change.id, change))
        .collect()
}

/// The feed delivered a version older than the one already agreed for the profile.
fn is_stale(change: &RemoteProfile, synced: Option<&SyncedEntry>) -> bool {
    synced.is_some_and(|synced| change.version < synced.version)
}

/// What to do about one profile. At least one of the three must be present.
pub fn reconcile(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    synced: Option<&SyncedEntry>,
) -> Vec<ReplicaAction> {
    let remote = remote.filter(|change| !is_stale(change, synced));
    let Some(id) = local
        .map(|profile| profile.id)
        .or_else(|| remote.map(|change| change.id))
        .or_else(|| synced.map(|entry| entry.profile.id))
    else {
        return Vec::new();
    };
    if remote.is_some_and(|change| change.body == RemoteBody::Unrecognized) {
        return vec![ReplicaAction::NeedsNewerClient { id }];
    }
    match synced {
        Some(synced) => with_history(local, remote, synced),
        None => without_history(local, remote),
    }
}

fn boxed(profile: &Profile) -> Box<Profile> {
    Box::new(profile.clone())
}

fn save_remote(
    change: &RemoteProfile,
    profile: &Profile,
    expected: Option<&Profile>,
) -> ReplicaAction {
    ReplicaAction::SaveLocal {
        version: change.version,
        profile: boxed(profile),
        expected: expected.map(boxed),
    }
}

fn mark_synced(change: &RemoteProfile, profile: &Profile) -> ReplicaAction {
    ReplicaAction::MarkSynced {
        version: change.version,
        profile: boxed(profile),
    }
}

/// Both disagree with a remote profile that has no agreed state: the remote wins.
fn take_remote(
    change: &RemoteProfile,
    remote: &Profile,
    local: Option<&Profile>,
) -> Vec<ReplicaAction> {
    match local {
        Some(local) if local == remote => vec![mark_synced(change, remote)],
        Some(local) => vec![
            save_remote(change, remote, Some(local)),
            ReplicaAction::Overwritten {
                id: local.id,
                discarded: boxed(local),
            },
        ],
        None => vec![save_remote(change, remote, None)],
    }
}

fn without_history(local: Option<&Profile>, remote: Option<&RemoteProfile>) -> Vec<ReplicaAction> {
    match (local, remote) {
        (None, None) => Vec::new(),
        (Some(local), None) => vec![ReplicaAction::Push {
            profile: boxed(local),
            expected: None,
        }],
        (_, Some(change)) => match &change.body {
            RemoteBody::Present(remote) => take_remote(change, remote, local),
            RemoteBody::Deleted => local
                .map(|local| ReplicaAction::Push {
                    profile: boxed(local),
                    expected: Some(change.version),
                })
                .into_iter()
                .collect(),
            RemoteBody::Unrecognized => Vec::new(),
        },
    }
}

#[derive(Clone, Copy)]
enum Side<'a> {
    Equal,
    Changed(&'a Profile),
    Deleted,
}

/// The remote side relative to `S`. A change carrying `S`'s version, or `S`'s content
/// at a newer version, is not a change of content.
fn remote_side<'a>(remote: Option<&'a RemoteProfile>, synced: &SyncedEntry) -> Side<'a> {
    let Some(change) = remote else {
        return Side::Equal;
    };
    match &change.body {
        _ if change.version == synced.version => Side::Equal,
        RemoteBody::Present(profile) if **profile == synced.profile => Side::Equal,
        RemoteBody::Present(profile) => Side::Changed(profile),
        RemoteBody::Deleted => Side::Deleted,
        RemoteBody::Unrecognized => Side::Equal,
    }
}

fn local_side<'a>(local: Option<&'a Profile>, synced: &SyncedEntry) -> Side<'a> {
    match local {
        None => Side::Deleted,
        Some(local) if *local == synced.profile => Side::Equal,
        Some(local) => Side::Changed(local),
    }
}

fn with_history(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    synced: &SyncedEntry,
) -> Vec<ReplicaAction> {
    let version = remote.map_or(synced.version, |change| change.version);
    match (
        local_side(local, synced),
        remote_side(remote, synced),
        local,
        remote,
    ) {
        (Side::Equal, Side::Equal, ..) if version == synced.version => Vec::new(),
        (Side::Equal, Side::Equal, ..) => vec![ReplicaAction::MarkSynced {
            version,
            profile: boxed(&synced.profile),
        }],
        (Side::Equal, Side::Changed(profile), _, Some(change)) => {
            vec![save_remote(change, profile, Some(&synced.profile))]
        }
        (Side::Equal, Side::Deleted, ..) => vec![ReplicaAction::DeleteLocal {
            id: synced.profile.id,
            expected: boxed(&synced.profile),
        }],
        (Side::Changed(local), Side::Equal | Side::Deleted, ..) => vec![ReplicaAction::Push {
            profile: boxed(local),
            expected: Some(version),
        }],
        (Side::Changed(local), Side::Changed(profile), _, Some(change)) => {
            take_remote(change, profile, Some(local))
        }
        (Side::Deleted, Side::Equal, ..) => vec![ReplicaAction::PushDelete {
            id: synced.profile.id,
            expected: version,
        }],
        (Side::Deleted, Side::Changed(profile), _, Some(change)) => {
            vec![save_remote(change, profile, None)]
        }
        (Side::Deleted, Side::Deleted, ..) => vec![ReplicaAction::Forget {
            id: synced.profile.id,
        }],
        _ => Vec::new(),
    }
}
