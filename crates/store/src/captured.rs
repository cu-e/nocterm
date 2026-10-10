use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures::{channel::oneshot, future::BoxFuture};
use nocterm_core::persist::WriteGate;

use crate::{Publication, Revision, SnapshotWriter, StoreError, persisted::Candidate};

/// A physically successful snapshot and its admission revision.
#[derive(Clone, Debug)]
pub struct Committed<T> {
    snapshot: T,
    revision: Revision,
}

impl<T> Committed<T> {
    pub(crate) fn new(snapshot: T, revision: Revision) -> Self {
        Self { snapshot, revision }
    }
    pub fn snapshot(&self) -> &T {
        &self.snapshot
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub(crate) fn hydrate(&mut self, apply: &mut impl FnMut(&mut T)) {
        apply(&mut self.snapshot);
    }
}

type Hydration<T> = Arc<dyn Fn(&mut T) + Send + Sync>;
struct Projection<T> {
    committed: Committed<T>,
    metadata: HashMap<TypeId, Hydration<T>>,
}
#[derive(Clone)]
pub(crate) struct DiskProjection<T>(Arc<Mutex<Projection<T>>>);

impl<T: Clone> DiskProjection<T> {
    pub(crate) fn new(initial: T) -> Self {
        Self(Arc::new(Mutex::new(Projection {
            committed: Committed::new(initial, 0),
            metadata: HashMap::new(),
        })))
    }
    pub(crate) fn read(&self) -> Committed<T> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .committed
            .clone()
    }
    pub(crate) fn record(&self, mut committed: Committed<T>) {
        let mut projection = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for apply in projection.metadata.values() {
            apply(&mut committed.snapshot);
        }
        projection.committed = committed;
    }
    pub(crate) fn hydrate(&self, key: TypeId, apply: Hydration<T>) {
        let mut projection = self.0.lock().unwrap_or_else(|e| e.into_inner());
        apply(&mut projection.committed.snapshot);
        projection.metadata.insert(key, apply);
    }
}

/// Prefix settles the previously active writer; final reports a successful drain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitPhase {
    Prefix,
    Final,
}

/// Exactly two successful-projection events; the producer never waits for consumers.
pub struct CapturedFeed<T> {
    prefix: oneshot::Receiver<Committed<T>>,
    final_commit: oneshot::Receiver<Committed<T>>,
}

/// Drain result and the last physically successful snapshot, even on later failure.
pub struct CapturedOutcome<T> {
    result: Result<(), StoreError>,
    committed: Committed<T>,
}

impl<T> CapturedOutcome<T> {
    pub fn result(&self) -> Result<(), StoreError> {
        self.result.clone()
    }
    pub fn committed(&self) -> &Committed<T> {
        &self.committed
    }
    pub fn into_result(self) -> Result<(), StoreError> {
        self.result
    }
}

/// Immutable shutdown work. No App, entity, owner callbacks or views survive here.
pub struct Captured<T> {
    durable: Committed<T>,
    active: Option<Candidate<T>>,
    final_candidate: Candidate<T>,
    publication: Publication,
    dirty: bool,
    writer: SnapshotWriter<T>,
    gate: WriteGate,
    projection: DiskProjection<T>,
}

impl<T: Clone + PartialEq + Send + 'static> Captured<T> {
    pub(crate) fn new(
        durable: Committed<T>,
        active: Option<Candidate<T>>,
        final_candidate: Candidate<T>,
        policy: (Publication, bool),
        writer: SnapshotWriter<T>,
        gate: WriteGate,
        projection: DiskProjection<T>,
    ) -> Self {
        Self {
            durable,
            active,
            final_candidate,
            publication: policy.0,
            dirty: policy.1,
            writer,
            gate,
            projection,
        }
    }
    pub fn durable(&self) -> &Committed<T> {
        &self.durable
    }
    pub fn active(&self) -> Option<Committed<T>> {
        self.active
            .as_ref()
            .map(|candidate| Committed::new(candidate.snapshot().clone(), candidate.revision()))
    }
    pub fn final_snapshot(&self) -> &T {
        self.final_candidate.snapshot()
    }
    pub fn final_revision(&self) -> Revision {
        self.final_candidate.revision()
    }

    pub fn run(self) -> BoxFuture<'static, CapturedOutcome<T>> {
        let (work, _feed) = self.run_observed();
        work
    }

    pub fn run_observed(mut self) -> (BoxFuture<'static, CapturedOutcome<T>>, CapturedFeed<T>) {
        let (prefix, prefix_receiver) = oneshot::channel();
        let (final_commit, final_receiver) = oneshot::channel();
        let work = Box::pin(async move {
            let gate = self.gate.clone();
            let _guard = gate.final_write().await;
            let prefix_snapshot = self.settle_active_prefix();
            let _ = prefix.send(prefix_snapshot);
            let result = self.write_final().await;
            self.final_candidate.finish(result.clone());
            if let Some(active) = &self.active {
                if self.publication == Publication::Optimistic {
                    active.finish_at(result.clone(), self.final_revision());
                } else {
                    active.finish(result.clone());
                }
            }
            let committed = self.projection.read();
            if result.is_ok() {
                let _ = final_commit.send(committed.clone());
            }
            CapturedOutcome { result, committed }
        });
        (
            work,
            CapturedFeed {
                prefix: prefix_receiver,
                final_commit: final_receiver,
            },
        )
    }

    fn settle_active_prefix(&self) -> Committed<T> {
        let committed = self.projection.read();
        if let Some(active) = &self.active
            && committed.revision() >= active.revision()
        {
            active.finish(Ok(()));
        }
        committed
    }

    async fn write_final(&mut self) -> Result<(), StoreError> {
        let committed = self.projection.read();
        let snapshot = self.final_candidate.snapshot().clone();
        if &snapshot != committed.snapshot()
            || (self.dirty && self.publication == Publication::Optimistic)
        {
            (self.writer)(snapshot.clone()).await?;
        }
        self.projection.record(Committed::new(
            snapshot.clone(),
            self.final_candidate.revision(),
        ));
        Ok(())
    }

    /// Save independent accepted edits before waiting for either dependency event.
    /// Derived projections use the same gate sequentially, with at most three writes.
    pub fn run_projected<P: Send + 'static>(
        mut self,
        feed: CapturedFeed<P>,
        mut project: impl Fn(CommitPhase, &Committed<P>) -> Result<Option<T>, StoreError>
        + Send
        + 'static,
    ) -> BoxFuture<'static, CapturedOutcome<T>> {
        Box::pin(async move {
            let gate = self.gate.clone();
            let _guard = gate.final_write().await;
            self.settle_active_prefix();
            let mut result = self.write_final().await;
            if result.is_ok() {
                self.finish_baseline(Ok(()));
            }
            result = self
                .project_event(feed.prefix, CommitPhase::Prefix, &mut project, result)
                .await;
            result = self
                .project_event(feed.final_commit, CommitPhase::Final, &mut project, result)
                .await;
            self.finish_baseline(result.clone());
            CapturedOutcome {
                result,
                committed: self.projection.read(),
            }
        })
    }

    fn finish_baseline(&self, result: Result<(), StoreError>) {
        self.final_candidate.finish(result.clone());
        if let Some(active) = &self.active {
            active.finish_at(result, self.final_revision());
        }
    }

    async fn project_event<P>(
        &mut self,
        event: oneshot::Receiver<Committed<P>>,
        phase: CommitPhase,
        project: &mut (impl Fn(CommitPhase, &Committed<P>) -> Result<Option<T>, StoreError> + Send),
        previous: Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let Ok(committed) = event.await else {
            return match phase {
                CommitPhase::Prefix => Err(previous.err().unwrap_or(StoreError::CompletionDropped)),
                CommitPhase::Final => previous,
            };
        };
        let Some(snapshot) = project(phase, &committed)? else {
            return previous;
        };
        if &snapshot != self.projection.read().snapshot() {
            (self.writer)(snapshot.clone()).await?;
        }
        self.projection
            .record(Committed::new(snapshot, self.final_revision()));
        self.finish_baseline(Ok(()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
