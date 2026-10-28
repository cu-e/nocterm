//! Three-way reconciliation of one profile: local `L`, remote `R` and the last agreed `S`.
//!
//! The rules are pure. They name what must happen and leave doing it, and the order
//! of side effects, to the caller. A remote change always wins a conflict; the local
//! edit it replaces is reported so the user can be told. A change older than the
//! agreed version is a late entry of the feed, never a newer state, and is ignored.
//!
//! A profile lives in one scope. Moving it to another is a deletion in the scope it
//! was agreed in followed by a creation in the new one, in that order: the profile is
//! never held in two scopes, and a refusal of the creation leaves it in the local
//! document to be sent to the personal scope.
use super::{RemoteBody, RemoteProfile, ReplicaScope, ReplicaState, ReplicaVersion, SyncedEntry};
use crate::profiles::{Profile, ProfileId, Profiles};
use nocterm_session::CredentialId;
use std::collections::{HashMap, HashSet};

/// One effect the reconciliation asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplicaAction {
    /// Write `profile` locally if the local copy still is `expected` (`None`: absent),
    /// then record agreement at `version`.
    SaveLocal {
        scope: ReplicaScope,
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
        scope: ReplicaScope,
        profile: Box<Profile>,
        expected: Option<ReplicaVersion>,
    },
    /// Send a deletion replacing the replica's `expected` version.
    PushDelete {
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
    },
    /// Both sides already hold `profile`; record agreement at `version`.
    MarkSynced {
        scope: ReplicaScope,
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
/// one, or with only changes older than its agreed version, is unchanged remotely. A profile the replica holds unreadably stays untouched
/// until a readable version of it arrives.
pub fn plan(
    local: &Profiles,
    changes: &[RemoteProfile],
    state: &ReplicaState,
) -> Vec<ReplicaAction> {
    let latest = latest_changes(changes, state);
    let mut remembered: Vec<ProfileId> = state.entries().map(|(id, _)| id).collect();
    remembered.sort_by_key(ToString::to_string);
    let mut seen = HashSet::new();
    let ids = local
        .iter()
        .map(|profile| profile.id)
        .chain(changes.iter().map(|change| change.id))
        .chain(remembered)
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    let mut actions: Vec<ReplicaAction> = ids
        .iter()
        .copied()
        .filter(|id| !state.is_unrecognized(*id) || latest.contains_key(id))
        .flat_map(|id| {
            reconcile_placed(
                local.get(id),
                latest.get(&id).copied(),
                state.entry(id),
                state.placement(id),
            )
        })
        .collect();
    let mut stale: Vec<ProfileId> = state
        .placements()
        .map(|(id, _)| id)
        .filter(|id| !seen.contains(id))
        .collect();
    stale.sort_by_key(ToString::to_string);
    actions.extend(stale.into_iter().map(|id| ReplicaAction::Forget { id }));
    actions
}

/// Of the `credentials` of profile `id` that are kept in the `from` space, the ones another
/// profile that stays there still uses. Such a secret is copied when `id` leaves, not moved:
/// moving it would take it from the profiles that remain. A profile never sent counts as staying
/// in the personal scope, where it is sent unless it is placed elsewhere.
pub fn shared_secrets(
    local: &Profiles,
    state: &ReplicaState,
    id: ProfileId,
    credentials: &[CredentialId],
    from: &ReplicaScope,
) -> Vec<CredentialId> {
    let staying: HashSet<CredentialId> = local
        .iter()
        .filter(|other| other.id != id)
        .filter(|other| {
            state
                .placement(other.id)
                .or_else(|| state.entry(other.id).map(|entry| &entry.scope))
                .unwrap_or(&ReplicaScope::Personal)
                == from
        })
        .flat_map(Profile::credentials)
        .collect();
    credentials
        .iter()
        .copied()
        .filter(|credential| staying.contains(credential))
        .collect()
}

/// The change that stands for each profile. A profile moved between scopes shows up twice,
/// as a tombstone where it was and as a live profile where it is; the live one stands, so
/// the move is never mistaken for a deletion. Otherwise the later change stands, except that
/// a change in the scope the profile was agreed in outranks one in another scope. Late entries
/// older than the agreed version never stand.
fn latest_changes<'a>(
    changes: &'a [RemoteProfile],
    state: &ReplicaState,
) -> HashMap<ProfileId, &'a RemoteProfile> {
    let mut latest: HashMap<ProfileId, &RemoteProfile> = HashMap::new();
    for change in changes {
        if is_stale(change, state.entry(change.id)) {
            continue;
        }
        let home = state.entry(change.id).map(|entry| &entry.scope);
        let stands = latest
            .get(&change.id)
            .is_none_or(|held| outranks(change, held, home));
        if stands {
            latest.insert(change.id, change);
        }
    }
    latest
}

fn outranks(new: &RemoteProfile, held: &RemoteProfile, home: Option<&ReplicaScope>) -> bool {
    fn rank(body: &RemoteBody) -> u8 {
        match body {
            RemoteBody::Deleted => 1,
            RemoteBody::Present(_) => 2,
            RemoteBody::Unrecognized => 3,
        }
    }
    let (new_rank, held_rank) = (rank(&new.body), rank(&held.body));
    if new_rank != held_rank {
        return new_rank > held_rank;
    }
    !(home == Some(&held.scope) && home != Some(&new.scope))
}

/// The feed delivered a version older than the one already agreed for the profile in the
/// same scope. Versions of different scopes are unrelated, so a copy in the scope a profile
/// was moved to is never stale against the version agreed where it was.
fn is_stale(change: &RemoteProfile, synced: Option<&SyncedEntry>) -> bool {
    synced.is_some_and(|synced| change.scope == synced.scope && change.version < synced.version)
}

/// What to do about one profile. At least one of the three must be present.
pub fn reconcile(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    synced: Option<&SyncedEntry>,
) -> Vec<ReplicaAction> {
    reconcile_placed(local, remote, synced, None)
}

/// [`reconcile`] for a profile the user asked to live in `wanted`: a local profile
/// goes there, from wherever it was agreed to live. A remote change still wins.
pub fn reconcile_placed(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    synced: Option<&SyncedEntry>,
    wanted: Option<&ReplicaScope>,
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
        Some(synced) => with_history(local, remote, synced, wanted),
        None => without_history(local, remote, wanted),
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
        scope: change.scope.clone(),
        version: change.version,
        profile: boxed(profile),
        expected: expected.map(boxed),
    }
}

fn mark_synced(change: &RemoteProfile, profile: &Profile) -> ReplicaAction {
    ReplicaAction::MarkSynced {
        scope: change.scope.clone(),
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

fn without_history(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    wanted: Option<&ReplicaScope>,
) -> Vec<ReplicaAction> {
    match (local, remote) {
        (None, None) => Vec::new(),
        (Some(local), None) => vec![ReplicaAction::Push {
            scope: wanted.cloned().unwrap_or(ReplicaScope::Personal),
            profile: boxed(local),
            expected: None,
        }],
        (_, Some(change)) => match &change.body {
            RemoteBody::Present(remote) => take_remote(change, remote, local),
            RemoteBody::Deleted => local
                .map(|local| {
                    let scope = wanted.unwrap_or(&change.scope);
                    ReplicaAction::Push {
                        scope: scope.clone(),
                        profile: boxed(local),
                        expected: (*scope == change.scope).then_some(change.version),
                    }
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

/// The actions that carry a local profile from the scope it was agreed in to `target`;
/// `None` when the remote changed the profile, which then wins over the move.
fn move_to(
    target: &ReplicaScope,
    local: Option<&Profile>,
    remote: Side<'_>,
    synced: &SyncedEntry,
    version: ReplicaVersion,
) -> Option<Vec<ReplicaAction>> {
    let local = local?;
    let create = ReplicaAction::Push {
        scope: target.clone(),
        profile: boxed(local),
        expected: None,
    };
    match remote {
        Side::Equal => Some(vec![
            ReplicaAction::PushDelete {
                scope: synced.scope.clone(),
                id: synced.profile.id,
                expected: version,
            },
            create,
        ]),
        Side::Deleted if *local != synced.profile => Some(vec![create]),
        Side::Deleted | Side::Changed(_) => None,
    }
}

fn with_history(
    local: Option<&Profile>,
    remote: Option<&RemoteProfile>,
    synced: &SyncedEntry,
    wanted: Option<&ReplicaScope>,
) -> Vec<ReplicaAction> {
    let version = remote.map_or(synced.version, |change| change.version);
    let scope = synced.scope.clone();
    let remote_state = remote_side(remote, synced);
    if let Some(target) = wanted.filter(|wanted| **wanted != synced.scope)
        && let Some(actions) = move_to(target, local, remote_state, synced, version)
    {
        return actions;
    }
    match (local_side(local, synced), remote_state, local, remote) {
        (Side::Equal, Side::Equal, ..) if version == synced.version => Vec::new(),
        (Side::Equal, Side::Equal, ..) => vec![ReplicaAction::MarkSynced {
            scope: remote.map_or(scope, |change| change.scope.clone()),
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
            scope,
            profile: boxed(local),
            expected: Some(version),
        }],
        (Side::Changed(local), Side::Changed(profile), _, Some(change)) => {
            take_remote(change, profile, Some(local))
        }
        (Side::Deleted, Side::Equal, ..) => vec![ReplicaAction::PushDelete {
            scope,
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
