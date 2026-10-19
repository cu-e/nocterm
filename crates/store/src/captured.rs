use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures::{channel::oneshot, future::BoxFuture};
use nocterm_core::persist::WriteGate;

use crate::{
    Publication, Revision, SnapshotWriter, StoreError,
    persisted::Candidate,
    worker::{
        DiskOperation,
        captured::{self as runner, CapturedPolicy},
    },
};

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

    pub fn run_observed(self) -> (BoxFuture<'static, CapturedOutcome<T>>, CapturedFeed<T>) {
        let (prefix, prefix_receiver) = oneshot::channel();
        let (final_commit, final_receiver) = oneshot::channel();
        let work = runner::run(ObservedSnapshot {
            captured: self,
            prefix: Some(prefix),
            final_commit: Some(final_commit),
            result: Ok(()),
            pending: true,
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

    fn final_operation(&self) -> DiskOperation<()> {
        self.snapshot_operation(self.final_candidate.snapshot().clone(), true)
    }
    fn snapshot_operation(&self, snapshot: T, baseline: bool) -> DiskOperation<()> {
        let write = &snapshot != self.projection.read().snapshot()
            || (baseline && self.dirty && self.publication == Publication::Optimistic);
        let writer = self.writer.clone();
        let saved = snapshot.clone();
        let revision = self.final_revision();
        let projection = self.projection.clone();
        DiskOperation::new(
            move || {
                Box::pin(async move {
                    if write {
                        writer(snapshot).await
                    } else {
                        Ok(())
                    }
                })
            },
            move |result| {
                if result.is_ok() {
                    projection.record(Committed::new(saved, revision));
                }
            },
        )
    }
    /// Save independent accepted edits before waiting for either dependency event.
    /// Derived projections use the same gate sequentially, with at most three writes.
    pub fn run_projected<P: Send + 'static>(
        self,
        feed: CapturedFeed<P>,
        project: impl Fn(CommitPhase, &Committed<P>) -> Result<Option<T>, StoreError> + Send + 'static,
    ) -> BoxFuture<'static, CapturedOutcome<T>> {
        runner::run(ProjectedSnapshot {
            captured: self,
            prefix: Some(feed.prefix),
            final_commit: Some(feed.final_commit),
            project,
            stage: ProjectionStage::Baseline,
            result: Ok(()),
        })
    }

    fn finish_baseline(&self, result: Result<(), StoreError>) {
        self.final_candidate.finish(result.clone());
        if let Some(active) = &self.active {
            active.finish_at(result, self.final_revision());
        }
    }
}

struct ObservedSnapshot<T> {
    captured: Captured<T>,
    prefix: Option<oneshot::Sender<Committed<T>>>,
    final_commit: Option<oneshot::Sender<Committed<T>>>,
    result: Result<(), StoreError>,
    pending: bool,
}
impl<T: Clone + PartialEq + Send + 'static> CapturedPolicy for ObservedSnapshot<T> {
    type Physical = ();
    type Output = CapturedOutcome<T>;
    fn gate(&self) -> WriteGate {
        self.captured.gate.clone()
    }
    fn joined(&mut self) {
        let committed = self.captured.settle_active_prefix();
        if let Some(prefix) = self.prefix.take() {
            let _ = prefix.send(committed);
        }
    }
    fn next_operation(&mut self) -> BoxFuture<'_, Option<DiskOperation<()>>> {
        let operation = self.pending.then(|| self.captured.final_operation());
        self.pending = false;
        Box::pin(async move { operation })
    }
    fn apply_outcome(&mut self, result: Result<(), StoreError>) {
        self.captured.final_candidate.finish(result.clone());
        if let Some(active) = &self.captured.active {
            if self.captured.publication == Publication::Optimistic {
                active.finish_at(result.clone(), self.captured.final_revision());
            } else {
                active.finish(result.clone());
            }
        }
        if result.is_ok()
            && let Some(final_commit) = self.final_commit.take()
        {
            let _ = final_commit.send(self.captured.projection.read());
        }
        self.result = result;
    }
    fn finish(self) -> CapturedOutcome<T> {
        CapturedOutcome {
            result: self.result,
            committed: self.captured.projection.read(),
        }
    }
}
#[derive(Clone, Copy)]
enum ProjectionStage {
    Baseline,
    Prefix,
    Final,
    Done,
}
struct ProjectedSnapshot<T, P, F> {
    captured: Captured<T>,
    prefix: Option<oneshot::Receiver<Committed<P>>>,
    final_commit: Option<oneshot::Receiver<Committed<P>>>,
    project: F,
    stage: ProjectionStage,
    result: Result<(), StoreError>,
}
impl<
    T: Clone + PartialEq + Send + 'static,
    P: Send + 'static,
    F: Fn(CommitPhase, &Committed<P>) -> Result<Option<T>, StoreError> + Send + 'static,
> ProjectedSnapshot<T, P, F>
{
    async fn projected(
        &mut self,
        phase: CommitPhase,
        event: oneshot::Receiver<Committed<P>>,
    ) -> Option<DiskOperation<()>> {
        let Ok(committed) = event.await else {
            if phase == CommitPhase::Prefix {
                self.result = Err(self
                    .result
                    .clone()
                    .err()
                    .unwrap_or(StoreError::CompletionDropped));
            }
            return None;
        };
        match (self.project)(phase, &committed) {
            Ok(Some(snapshot)) => Some(self.captured.snapshot_operation(snapshot, false)),
            Ok(None) => None,
            Err(error) => {
                self.result = Err(error);
                None
            }
        }
    }
}
impl<
    T: Clone + PartialEq + Send + 'static,
    P: Send + 'static,
    F: Fn(CommitPhase, &Committed<P>) -> Result<Option<T>, StoreError> + Send + 'static,
> CapturedPolicy for ProjectedSnapshot<T, P, F>
{
    type Physical = ();
    type Output = CapturedOutcome<T>;
    fn gate(&self) -> WriteGate {
        self.captured.gate.clone()
    }
    fn joined(&mut self) {
        self.captured.settle_active_prefix();
    }
    fn next_operation(&mut self) -> BoxFuture<'_, Option<DiskOperation<()>>> {
        Box::pin(async move {
            loop {
                let next = match self.stage {
                    ProjectionStage::Baseline => {
                        self.stage = ProjectionStage::Prefix;
                        Some(self.captured.final_operation())
                    }
                    ProjectionStage::Prefix => {
                        self.stage = ProjectionStage::Final;
                        let event = self.prefix.take()?;
                        self.projected(CommitPhase::Prefix, event).await
                    }
                    ProjectionStage::Final => {
                        self.stage = ProjectionStage::Done;
                        let event = self.final_commit.take()?;
                        self.projected(CommitPhase::Final, event).await
                    }
                    ProjectionStage::Done => return None,
                };
                if next.is_some() {
                    return next;
                }
            }
        })
    }
    fn apply_outcome(&mut self, result: Result<(), StoreError>) {
        if result.is_ok() {
            self.captured.finish_baseline(Ok(()));
        }
        self.result = result;
    }
    fn finish(self) -> CapturedOutcome<T> {
        self.captured.finish_baseline(self.result.clone());
        CapturedOutcome {
            result: self.result,
            committed: self.captured.projection.read(),
        }
    }
}

#[cfg(test)]
mod tests;
