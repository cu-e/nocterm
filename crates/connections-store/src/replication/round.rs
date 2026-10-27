//! One synchronization pass: read remote changes, reconcile, apply, advance the cursor.
//!
//! Every action is persisted in the agreed state as soon as it succeeds, and the
//! cursor moves only after a pass in which nothing was left undone. A pass that
//! is interrupted or retried therefore re-reads changes it already handled; those
//! reconcile to nothing because the agreed state already holds them.
use super::{ProfileReplication, ReplicationEvent, ReplicationFailure};
use crate::{ConnectionStore, ConnectionsError};
use gpui::{AsyncApp, Entity, WeakEntity};
use nocterm_connections_model::{
    profiles::{Profile, ProfileError, ProfileId, Profiles},
    replica::{
        ProfileReplica, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaScope, ReplicaState,
        ReplicaVersion, SyncedEntry,
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
    Failed(ReplicationFailure),
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
        error => halt(error.into()),
    }
}

fn remote_flow(error: ReplicaError) -> Flow {
    match error {
        ReplicaError::Conflict { .. } => Flow::Conflict,
        ReplicaError::Forbidden { .. } => Flow::Skip(error.into()),
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
        if self.apply(actions, cx).await? {
            return Ok(true);
        }
        self.record(cx, move |state| state.advance(cursor))?;
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
        let mut after = self
            .state
            .read_with(cx, |state, _| state.visible().cursor().cloned());
        let mut changes = Vec::new();
        loop {
            let page = self
                .replica
                .changes(after.clone())
                .await
                .map_err(remote_stop)?;
            changes.extend(page.changes);
            let stalled = after.as_ref() == Some(&page.cursor);
            after = Some(page.cursor);
            if !page.more || stalled {
                break;
            }
        }
        let cursor = after.unwrap_or_else(|| ReplicaCursor::new(""));
        Ok((changes, cursor))
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

    /// Whether a conflict left something undone.
    async fn apply(&self, actions: Vec<ReplicaAction>, cx: &mut AsyncApp) -> Result<bool, Stop> {
        let mut skipped = HashSet::new();
        let mut retry = false;
        for action in actions {
            let id = action.id();
            if skipped.contains(&id) {
                continue;
            }
            match self.execute(action, cx).await {
                Ok(()) => {}
                Err(Flow::Conflict) => {
                    retry = true;
                    skipped.insert(id);
                }
                Err(Flow::Skip(failure)) => {
                    skipped.insert(id);
                    self.emit(ReplicationEvent::Failed(failure), cx)?;
                }
                Err(Flow::Stop(stop)) => return Err(stop),
            }
        }
        Ok(retry)
    }

    async fn execute(&self, action: ReplicaAction, cx: &mut AsyncApp) -> Result<(), Flow> {
        match action {
            ReplicaAction::SaveLocal {
                scope,
                version,
                profile,
                expected,
            } => {
                self.save_local(scope, version, *profile, expected.map(|p| *p), cx)
                    .await
            }
            ReplicaAction::DeleteLocal { id, expected } => {
                self.delete_local(id, *expected, cx).await
            }
            ReplicaAction::Push {
                scope,
                profile,
                expected,
            } => self.push(scope, *profile, expected, cx).await,
            ReplicaAction::PushDelete {
                scope,
                id,
                expected,
            } => self.push_delete(scope, id, expected, cx).await,
            ReplicaAction::MarkSynced {
                scope,
                version,
                profile,
            } => Ok(self.agree(scope, version, *profile, cx)?),
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
        scope: ReplicaScope,
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
        Ok(self.agree(scope, version, agreed, cx)?)
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
        scope: ReplicaScope,
        profile: Profile,
        expected: Option<ReplicaVersion>,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        let version = self
            .replica
            .put(scope.clone(), profile.clone(), expected)
            .await
            .map_err(remote_flow)?;
        Ok(self.agree(scope, version, profile, cx)?)
    }

    async fn push_delete(
        &self,
        scope: ReplicaScope,
        id: ProfileId,
        expected: ReplicaVersion,
        cx: &mut AsyncApp,
    ) -> Result<(), Flow> {
        self.replica
            .delete(scope, id, expected)
            .await
            .map_err(remote_flow)?;
        Ok(self.record(cx, move |state| state.forget(id))?)
    }

    fn agree(
        &self,
        scope: ReplicaScope,
        version: ReplicaVersion,
        profile: Profile,
        cx: &mut AsyncApp,
    ) -> Result<(), Stop> {
        let id = profile.id;
        let entry = SyncedEntry {
            scope,
            version,
            profile,
        };
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

    fn emit(&self, event: ReplicationEvent, cx: &mut AsyncApp) -> Result<(), Stop> {
        self.events.update(cx, |_, cx| cx.emit(event)).map_err(gone)
    }
}
