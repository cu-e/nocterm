//! Keeps the saved profiles and a remote replica of them in agreement.
//!
//! The local document stays the one the application reads and writes through
//! [`ConnectionStore`]. This entity watches its committed publications, reads the
//! replica's changes, reconciles the three sides with the pure rules of
//! `connections-model` and applies the result: remote changes through the store's
//! checked writes, local ones through the replica port. What the two sides last
//! agreed on is a [`ReplicaState`] document persisted through `nocterm_store`, so a
//! restart continues where it stopped.
mod round;
mod status;

use crate::ConnectionStore;
use futures::StreamExt as _;
use gpui::{App, Context, Entity, EventEmitter, Subscription, Task};
use nocterm_connections_model::{
    profiles::ProfileId,
    replica::{ProfileReplica, ReplicaScope, ReplicaState},
};
use nocterm_store::{Persisted, Publication, SnapshotWriter};
use round::{Outcome, Round};
use std::{collections::HashSet, sync::Arc};

pub use status::{ReplicationEvent, ReplicationFailure, ReplicationStatus};

/// Pending writes of the agreed-state document; its edits are coalesced snapshots.
const STATE_WRITES: usize = 8;

pub struct ProfileReplication {
    store: Entity<ConnectionStore>,
    state: Entity<Persisted<ReplicaState>>,
    replica: Option<Arc<dyn ProfileReplica>>,
    status: ReplicationStatus,
    run: Option<Task<()>>,
    rerun: bool,
    watcher: Option<Task<()>>,
    /// Remote changes the local document refused and that were already reported; the
    /// cursor stays behind them, so each pass reads them again but says so once.
    refused: HashSet<ProfileId>,
    _publications: Subscription,
}

impl EventEmitter<ReplicationEvent> for ProfileReplication {}

impl ProfileReplication {
    /// `state` is the document built by [`Self::state_document`] and registered
    /// with the shutdown coordinator by the application.
    pub fn new(
        store: Entity<ConnectionStore>,
        state: Entity<Persisted<ReplicaState>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let publications = store.update(cx, |store, cx| store.publications(cx));
        let subscription = cx.observe(&publications, |this, _, cx| this.request_sync(cx));
        Self {
            store,
            state,
            replica: None,
            status: ReplicationStatus::Detached,
            run: None,
            rerun: false,
            watcher: None,
            refused: HashSet::new(),
            _publications: subscription,
        }
    }

    /// The agreed-state document. It publishes optimistically: the replication reads
    /// its own latest edits back while the disk catches up.
    pub fn state_document(
        initial: ReplicaState,
        writer: SnapshotWriter<ReplicaState>,
    ) -> Persisted<ReplicaState> {
        Persisted::new(initial, writer, Publication::Optimistic, STATE_WRITES)
    }

    /// Starts synchronizing with `replica`, replacing a previously attached one.
    /// The agreed state is kept: a restart continues with the same replica.
    pub fn attach(&mut self, replica: Arc<dyn ProfileReplica>, cx: &mut Context<Self>) {
        self.stop(cx);
        let mut changes = replica.watch();
        self.watcher = Some(cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.request_sync(cx)).is_err() {
                    break;
                }
            }
        }));
        self.replica = Some(replica);
        self.start(cx);
    }

    /// Stops synchronizing and forgets what was agreed with that replica: the
    /// agreement and the feed position mean nothing to another account's replica.
    /// Local profiles are left as they are.
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.state.update(cx, |state, cx| {
            if let Err(error) = state.update_latest(|state| *state = ReplicaState::default(), cx) {
                tracing::error!(%error, "could not reset the profile replica state");
            }
        });
        self.set_status(ReplicationStatus::Detached, cx);
    }

    pub fn status(&self) -> &ReplicationStatus {
        &self.status
    }

    /// Where the profile was last agreed to live, when it is replicated.
    pub fn scope_of(&self, id: ProfileId, cx: &App) -> Option<ReplicaScope> {
        let state = self.state.read(cx).visible();
        state.entry(id).map(|entry| entry.scope.clone())
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.replica = None;
        self.watcher = None;
        self.run = None;
        self.rerun = false;
        cx.notify();
    }

    fn request_sync(&mut self, cx: &mut Context<Self>) {
        if self.replica.is_none() {
            return;
        }
        if self.run.is_some() {
            self.rerun = true;
        } else {
            self.start(cx);
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(replica) = self.replica.clone() else {
            return;
        };
        let round = Round::new(
            replica,
            self.store.clone(),
            self.state.clone(),
            cx.weak_entity(),
        );
        self.set_status(ReplicationStatus::Syncing, cx);
        self.run = Some(cx.spawn(async move |this, cx| {
            let outcome = round.run(cx).await;
            if let Some(outcome) = outcome {
                this.update(cx, |this, cx| this.finish(outcome, cx)).ok();
            }
        }));
    }

    /// Reports a profile the pass skipped. A refused remote change is read again by
    /// every pass until it can be applied, but is announced only the first time.
    fn announce_skip(
        &mut self,
        id: ProfileId,
        failure: ReplicationFailure,
        cx: &mut Context<Self>,
    ) {
        let first =
            !matches!(failure, ReplicationFailure::Rejected { .. }) || self.refused.insert(id);
        if first {
            cx.emit(ReplicationEvent::Failed(failure));
        }
    }

    /// A pass left nothing refused behind it: later refusals are news again.
    fn forget_refusals(&mut self) {
        self.refused.clear();
    }

    fn finish(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        self.run = None;
        let status = match outcome {
            Outcome::Synced => ReplicationStatus::Synced,
            Outcome::NeedsNewerClient => ReplicationStatus::NeedsNewerClient,
            Outcome::Offline => ReplicationStatus::Offline,
            Outcome::Failed(failure) => {
                cx.emit(ReplicationEvent::Failed(failure.clone()));
                ReplicationStatus::Failed(failure)
            }
        };
        self.set_status(status, cx);
        if std::mem::take(&mut self.rerun) {
            self.start(cx);
        }
    }

    fn set_status(&mut self, status: ReplicationStatus, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
