//! One synchronization pass: read remote changes, reconcile, apply, advance the cursor.
//!
//! Every action is persisted in the agreed state as soon as it succeeds, and the
//! cursor moves only after a pass in which nothing was left undone: not after a
//! conflict, and not after a remote change the local document refused. A pass that
//! is interrupted, retried or refused re-reads changes it already handled; those
//! reconcile to nothing because the agreed state already holds them, while a refused
//! change is offered again instead of being lost.
use super::{ProfileReplication, ReplicationEvent, ReplicationFailure};
use crate::{ConnectionStore, ConnectionsError};
use gpui::{AsyncApp, Entity, WeakEntity};
use nocterm_connections_model::{
    profiles::{Profile, ProfileError, ProfileId, Profiles},
    replica::{
        ProfileReplica, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaState, ReplicaVersion,
        SyncedEntry,
        rules::{self, ReplicaAction},
    },
};
use nocterm_store::Persisted;
use std::{collections::HashSet, sync::Arc};

/// Passes tried before a profile that keeps changing under us is reported.
pub(super) const MAX_ATTEMPTS: usize = 3;

pub(super) enum Outcome {
    Synced,
    NeedsNewerClient,
    Offline,
    Locked,
    Failed(ReplicationFailure),
}

/// What applying the planned actions left behind.
enum Applied {
    Done,
    /// Nothing is left to retry, but the replica refused a profile that stays local.
    Refused,
    /// A conflict left something undone: plan again from the same cursor.
    Retry,
    /// A remote change could not be written locally: it stays ahead of the cursor.
    Deferred,
}

/// The pass cannot continue.
enum Stop {
    Halt(Outcome),
    /// The application or the owning entity is gone.
    Gone,
}

/// Why one action did not finish.
enum Flow {
    /// One side changed under the action; read and reconcile again.
    Conflict,
    /// This profile cannot be synchronized; the others still can.
    Skip(ReplicationFailure),
    /// The replica refused the profile and the refusal was already dealt with and
    /// reported; leave the rest of its actions undone.
    Refused,
    Stop(Stop),
}

impl From<Stop> for Flow {
    fn from(stop: Stop) -> Self {
        Self::Stop(stop)
    }
}

fn gone<E>(_: E) -> Stop {
    Stop::Gone
}

fn halt(failure: ReplicationFailure) -> Stop {
    Stop::Halt(Outcome::Failed(failure))
}

fn remote_stop(error: ReplicaError) -> Stop {
    match error {
        ReplicaError::Unavailable => Stop::Halt(Outcome::Offline),
        ReplicaError::Locked | ReplicaError::Stale => Stop::Halt(Outcome::Locked),
        error => halt(error.into()),
    }
}

fn remote_flow(error: ReplicaError) -> Flow {
    match error {
        ReplicaError::Conflict { .. } => Flow::Conflict,
        error => remote_stop(error).into(),
    }
}

fn local_flow(id: ProfileId, error: ConnectionsError) -> Flow {
    match &error {
        ConnectionsError::Profile(ProfileError::Conflict(_) | ProfileError::MissingProfile(_)) => {
            Flow::Conflict
        }
        ConnectionsError::Profile(_) => {
            Flow::Skip(ReplicationFailure::Rejected { id, source: error })
        }
        _ => halt(ReplicationFailure::Local(error)).into(),
    }
}

/// The action brings a remote change into the local document.
fn writes_remote_change(action: &ReplicaAction) -> bool {
    matches!(
        action,
        ReplicaAction::SaveLocal { .. } | ReplicaAction::DeleteLocal { .. }
    )
}

pub(super) struct Round {
    replica: Arc<dyn ProfileReplica>,
    store: Entity<ConnectionStore>,
    state: Entity<Persisted<ReplicaState>>,
    events: WeakEntity<ProfileReplication>,
}

impl Round {
    pub(super) fn new(
        replica: Arc<dyn ProfileReplica>,
        store: Entity<ConnectionStore>,
        state: Entity<Persisted<ReplicaState>>,
        events: WeakEntity<ProfileReplication>,
    ) -> Self {
        Self {
            replica,
            store,
            state,
            events,
        }
    }

    /// `None` when the application went away before the pass finished.
    pub(super) async fn run(self, cx: &mut AsyncApp) -> Option<Outcome> {
        for _ in 0..MAX_ATTEMPTS {
            match self.attempt(cx).await {
                Ok(true) => {}
                Ok(false) => return Some(self.finished(cx)),
                Err(Stop::Halt(outcome)) => return Some(outcome),
                Err(Stop::Gone) => return None,
            }
        }
        Some(Outcome::Failed(ReplicationFailure::Conflicts))
    }

    fn finished(&self, cx: &mut AsyncApp) -> Outcome {
        let unreadable = self
            .state
            .read_with(cx, |state, _| state.visible().has_unrecognized());
        if unreadable {
            Outcome::NeedsNewerClient
        } else {
            Outcome::Synced
        }
    }

    /// Whether the pass must be repeated.
    async fn attempt(&self, cx: &mut AsyncApp) -> Result<bool, Stop> {
        self.require_readable_local(cx)?;
        let (changes, cursor) = self.fetch(cx).await?;
        let actions = self.plan(&changes, cx)?;
        match self.apply(actions, cx).await? {
            Applied::Retry => return Ok(true),
            Applied::Refused => self.record(cx, move |state| state.advance(cursor))?,
            Applied::Done => {
                self.record(cx, move |state| state.advance(cursor))?;
                self.events
                    .update(cx, |this, _| this.forget_refusals())
                    .map_err(gone)?;
            }
            Applied::Deferred => {}
        }
        Ok(false)
    }

    fn require_readable_local(&self, cx: &mut AsyncApp) -> Result<(), Stop> {
        let unreadable = self
            .store
            .read_with(cx, |store, _| store.load_error().is_some());
        if unreadable {
            return Err(halt(ReplicationFailure::LocalUnavailable));
        }
        Ok(())
    }

    async fn fetch(&self, cx: &mut AsyncApp) -> Result<(Vec<RemoteProfile>, ReplicaCursor), Stop> {
        let after = self
            .state
            .read_with(cx, |state, _| state.visible().cursor().cloned());
        let replica = self.replica.clone();
        let executor = cx.background_executor().clone();
        let acquisition = executor.spawn(async move { super::feed::fetch(replica, after).await });
        let deadline = executor.timer(std::time::Duration::from_secs(120));
        futures::pin_mut!(acquisition, deadline);
        match futures::future::select(acquisition, deadline).await {
            futures::future::Either::Left((result, _)) => result.map_err(|error| match error {
                super::feed::FetchError::Remote(error) => remote_stop(error),
                super::feed::FetchError::Invalid(error) => halt(error.into()),
            }),
            futures::future::Either::Right(_) => {
                Err(halt(super::ReplicationFeedError::TimedOut.into()))
            }
        }
    }

    fn plan(
        &self,
        changes: &[RemoteProfile],
        cx: &mut AsyncApp,
    ) -> Result<Vec<ReplicaAction>, Stop> {
        let local: Profiles = self
            .store
            .read_with(cx, |store, cx| store.profiles(cx).clone());
        let state = self.state.read_with(cx, |state, _| state.visible().clone());
        Ok(rules::plan(&local, changes, &state))
    }

    async fn apply(&self, actions: Vec<ReplicaAction>, cx: &mut AsyncApp) -> Result<Applied, Stop> {
        let mut skipped = HashSet::new();
        let mut applied = Applied::Done;
        let mut refused = false;
        for action in actions {
            let id = action.id();
            if skipped.contains(&id) {
                continue;
            }
            let from_remote = writes_remote_change(&action);
            match self.execute(action, cx).await {
                Ok(()) => {}
                Err(Flow::Conflict) => {
                    applied = Applied::Retry;
                    skipped.insert(id);
                }
                Err(Flow::Refused) => {
                    refused = true;
                    skipped.insert(id);
                }
                Err(Flow::Skip(failure)) => {
                    skipped.insert(id);
                    if from_remote && matches!(applied, Applied::Done) {
                        applied = Applied::Deferred;
                    }
                    self.skipped(id, failure, cx)?;
                }
                Err(Flow::Stop(stop)) => return Err(stop),
            }
        }
        Ok(match applied {
            Applied::Done if refused => Applied::Refused,
            other => other,
        })
    }

    async fn execute(&self, action: ReplicaAction, cx: &mut AsyncApp) -> Result<(), Flow> {
        match action {
            ReplicaAction::SaveLocal {
                version,
                profile,
                expected,
            } => {
                self.save_local(version, *profile, expected.map(|p| *p), cx)
                    .await
            }
            ReplicaAction::DeleteLocal { id, expected } => {
                self.delete_local(id, *expected, cx).await
            }
            ReplicaAction::Push { profile, expected } => self.push(*profile, expected, cx).await,
            ReplicaAction::PushDelete { id, expected } => self.push_delete(id, expected, cx).await,
            ReplicaAction::MarkSynced { version, profile } => {
                Ok(self.agree(version, *profile, cx)?)
            }
            ReplicaAction::Forget { id } => Ok(self.record(cx, move |state| state.forget(id))?),
            ReplicaAction::Overwritten { id, discarded } => {
                Ok(self.emit(ReplicationEvent::Overwritten { id, discarded }, cx)?)
            }
            ReplicaAction::NeedsNewerClient { id } => {
                Ok(self.record(cx, move |state| state.mark_unrecognized(id))?)
            }
        }
    }

    async fn save_local(
        &self,
        version: ReplicaVersion,
        profile: Profile,
        expected: Option<Profile>,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        let id = profile.id;
        let agreed = profile.clone();
        let saving = self.store.update(cx, |store, cx| {
            store.save_profile_checked(profile, expected, cx)
        });
        saving.await.map_err(|error| local_flow(id, error))?;
        Ok(self.agree(version, agreed, cx)?)
    }

    /// The comparison and the deletion happen in one synchronous step, so an edit
    /// made after planning is not deleted unseen.
    async fn delete_local(
        &self,
        id: ProfileId,
        expected: Profile,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        let deleting = self.store.update(cx, |store, cx| {
            (store.profiles(cx).get(id) == Some(&expected)).then(|| store.delete_profile(id, cx))
        });
        let Some(deleting) = deleting else {
            return Err(Flow::Conflict);
        };
        deleting.await.map_err(|error| local_flow(id, error))?;
        Ok(self.record(cx, move |state| state.forget(id))?)
    }

    async fn push(
        &self,
        profile: Profile,
        expected: Option<ReplicaVersion>,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        let id = profile.id;
        match self.replica.put(profile.clone(), expected).await {
            Ok(version) => Ok(self.agree(version, profile, cx)?),
            Err(ReplicaError::Forbidden { .. }) => self.refused(id, profile, cx).await,
            Err(error) => Err(remote_flow(error)),
        }
    }

    async fn push_delete(
        &self,
        id: ProfileId,
        expected: ReplicaVersion,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        match self.replica.delete(id, expected).await {
            Ok(_) => Ok(self.record(cx, move |state| state.forget(id))?),
            Err(ReplicaError::Forbidden { .. }) => {
                let agreed = self
                    .state
                    .read_with(cx, |state, _| state.visible().entry(id).cloned());
                match agreed {
                    Some(entry) => self.refused(id, entry.profile, cx).await,
                    None => Err(Flow::Refused),
                }
            }
            Err(error) => Err(remote_flow(error)),
        }
    }

    /// The account may not write `profile`. The user is told once, and the local profile
    /// goes back to what was last agreed, so nothing is sent again. A profile never agreed
    /// stays local.
    async fn refused(
        &self,
        id: ProfileId,
        profile: Profile,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        let agreed = self
            .state
            .read_with(cx, |state, _| state.visible().entry(id).cloned());
        let restored = agreed.is_some();
        if let Some(entry) = agreed {
            let current = self
                .store
                .read_with(cx, |store, cx| store.profiles(cx).get(id).cloned());
            if current.as_ref() != Some(&entry.profile) {
                let restoring = self.store.update(cx, |store, cx| {
                    store.save_profile_checked(entry.profile, current, cx)
                });
                restoring.await.map_err(|error| local_flow(id, error))?;
            }
        }
        let event = ReplicationEvent::Refused {
            profile: Box::new(profile),
            restored,
        };
        self.events
            .update(cx, |this, cx| this.announce_refusal(id, event, cx))
            .map_err(gone)?;
        Err(Flow::Refused)
    }

    /// The replica's version stands, whether this client sent it, wrote it here or both
    /// already held it.
    fn agree(
        &self,
        version: ReplicaVersion,
        profile: Profile,
        cx: &mut AsyncApp,
    ) -> Result<(), Stop> {
        let id = profile.id;
        let entry = SyncedEntry { version, profile };
        self.record(cx, move |state| state.agree(id, entry))
    }

    fn record(
        &self,
        cx: &mut AsyncApp,
        edit: impl FnOnce(&mut ReplicaState) + 'static,
    ) -> Result<(), Stop> {
        self.state
            .update(cx, |state, cx| state.update_latest(edit, cx))
            .map(|_| ())
            .map_err(|error| halt(ReplicationFailure::State(error)))
    }

    fn skipped(
        &self,
        id: ProfileId,
        failure: ReplicationFailure,
        cx: &mut AsyncApp,
    ) -> Result<(), Stop> {
        self.events
            .update(cx, |this, cx| this.announce_skip(id, failure, cx))
            .map_err(gone)
    }

    fn emit(&self, event: ReplicationEvent, cx: &mut AsyncApp) -> Result<(), Stop> {
        self.events.update(cx, |_, cx| cx.emit(event)).map_err(gone)
    }
}
