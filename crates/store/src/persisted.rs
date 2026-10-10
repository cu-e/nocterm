use std::sync::{Arc, Mutex};

use futures::{channel::oneshot, future::BoxFuture};
use gpui::{Context, Entity, Task};
use nocterm_core::persist::{Rejected, WriteGate, WriteQueue};

use crate::{Captured, Committed, StoreError, captured::DiskProjection};

type PublicationHook<T> = Box<dyn Fn(&T, &T, &mut gpui::App)>;

pub type Revision = u64;
/// The owner supplies encoding and I/O without requiring an application handle.
pub type SnapshotWriter<T> =
    Arc<dyn Fn(T) -> BoxFuture<'static, Result<(), StoreError>> + Send + Sync>;

/// Whether accepted edits become visible before disk success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    /// Coalesce pending snapshots. Failed writes preserve the desired view.
    Optimistic,
    /// Apply each edit to the last successfully written document.
    Durable,
}

/// Resolves when this edit (or a later optimistic snapshot containing it) is durable.
pub struct Receipt(oneshot::Receiver<Result<Revision, StoreError>>);

impl Receipt {
    pub async fn wait(self) -> Result<Revision, StoreError> {
        self.0.await.unwrap_or(Err(StoreError::CompletionDropped))
    }
}

#[derive(Clone)]
pub(crate) struct Completion(Arc<Mutex<Option<oneshot::Sender<Result<Revision, StoreError>>>>>);

impl Completion {
    fn new() -> (Self, Receipt) {
        let (sender, receiver) = oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(sender)))), Receipt(receiver))
    }

    pub(crate) fn finish(&self, result: Result<Revision, StoreError>) {
        if let Some(sender) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(result);
        }
    }
}

type Edit<T> = Box<dyn FnOnce(&mut T) -> Result<(), StoreError>>;
struct Change<T> {
    revision: Revision,
    edit: Option<Edit<T>>,
    completion: Completion,
}

#[derive(Clone)]
pub(crate) struct Candidate<T> {
    revision: Revision,
    snapshot: T,
    completions: Vec<Completion>,
}

impl<T> Candidate<T> {
    pub(crate) fn new(revision: Revision, snapshot: T, completions: Vec<Completion>) -> Self {
        Self {
            revision,
            snapshot,
            completions,
        }
    }
    pub(crate) fn snapshot(&self) -> &T {
        &self.snapshot
    }
    pub(crate) fn revision(&self) -> Revision {
        self.revision
    }
    pub(crate) fn finish(&self, result: Result<(), StoreError>) {
        self.finish_at(result, self.revision);
    }
    pub(crate) fn finish_at(&self, result: Result<(), StoreError>, revision: Revision) {
        for completion in &self.completions {
            completion.finish(result.clone().map(|()| revision));
        }
    }
}

/// An entity owning desired/durable projections and one bounded ordered worker.
/// Its fields remain private: callers submit edits and read immutable projections.
pub struct Persisted<T> {
    desired: T,
    durable: Committed<T>,
    revision: Revision,
    queue: WriteQueue<Change<T>>,
    candidate: Option<Candidate<T>>,
    writer: SnapshotWriter<T>,
    gate: WriteGate,
    task: Option<Task<()>>,
    error: Option<StoreError>,
    publication: Publication,
    latest_pending: bool,
    hook: Option<PublicationHook<T>>,
    projection: DiskProjection<T>,
}

impl<T: Clone + PartialEq + Send + 'static> Persisted<T> {
    pub fn new(
        initial: T,
        writer: SnapshotWriter<T>,
        publication: Publication,
        capacity: usize,
    ) -> Self {
        Self {
            desired: initial.clone(),
            durable: Committed::new(initial.clone(), 0),
            projection: DiskProjection::new(initial),
            hook: None,
            revision: 0,
            queue: WriteQueue::new(capacity),
            candidate: None,
            writer,
            gate: WriteGate::default(),
            task: None,
            error: None,
            publication,
            latest_pending: false,
        }
    }

    /// Install a synchronous derived-state transition before accepting edits.
    /// The hook must not access this same entity or start I/O.
    pub fn with_publication_hook(
        mut self,
        hook: impl Fn(&T, &T, &mut gpui::App) + 'static,
    ) -> Self {
        self.hook = Some(Box::new(hook));
        self
    }

    pub fn visible(&self) -> &T {
        match self.publication {
            Publication::Optimistic => &self.desired,
            Publication::Durable => self.durable.snapshot(),
        }
    }

    pub fn durable(&self) -> &T {
        self.durable.snapshot()
    }

    pub fn is_closing(&self) -> bool {
        self.queue.is_closing()
    }

    pub fn is_dirty(&self) -> bool {
        self.candidate.is_some()
            || match self.publication {
                Publication::Optimistic => &self.desired != self.durable.snapshot(),
                Publication::Durable => !self.queue.is_empty(),
            }
    }

    pub fn error(&self) -> Option<&StoreError> {
        self.error.as_ref()
    }

    /// Queue an edit without exposing mutable state. Rejection changes no state.
    pub fn update(
        &mut self,
        edit: impl FnOnce(&mut T) + 'static,
        cx: &mut Context<Self>,
    ) -> Result<Receipt, StoreError> {
        self.update_checked(
            move |snapshot| {
                edit(snapshot);
                Ok(())
            },
            cx,
        )
    }

    /// Validate an ordered mutation against its latest successfully saved snapshot.
    /// Rejected mutations resolve their receipt without writing or publishing state.
    pub fn update_checked(
        &mut self,
        edit: impl FnOnce(&mut T) -> Result<(), StoreError> + 'static,
        cx: &mut Context<Self>,
    ) -> Result<Receipt, StoreError> {
        if self.queue.is_closing() {
            return Err(StoreError::Closing);
        }
        let (completion, receipt) = Completion::new();
        let mut desired = self.desired.clone();
        let edit: Option<Edit<T>> = if self.publication == Publication::Optimistic {
            edit(&mut desired)?;
            if desired == self.desired && !self.is_dirty() {
                completion.finish(Ok(self.revision));
                return Ok(receipt);
            }
            None
        } else {
            Some(Box::new(edit))
        };
        let revision = self.revision
            + u64::from(self.publication == Publication::Durable || desired != self.desired);
        self.queue
            .push(Change {
                revision,
                edit,
                completion,
            })
            .map_err(|e| match e {
                Rejected::Closing => StoreError::Closing,
                Rejected::Full => StoreError::Full,
            })?;
        self.revision = revision;
        self.desired = desired;
        cx.notify();
        if self.task.is_none() {
            self.start(cx);
        }
        Ok(receipt)
    }

    /// Admit a latest desired snapshot without allocating a receipt or queue slot.
    /// Intended for optimistic presentation state, such as resize events. It
    /// remains bounded by one marker even while disk or the executor is blocked.
    pub fn update_latest(
        &mut self,
        edit: impl FnOnce(&mut T),
        cx: &mut Context<Self>,
    ) -> Result<Revision, StoreError> {
        if self.queue.is_closing() {
            return Err(StoreError::Closing);
        }
        if self.publication != Publication::Optimistic {
            return Err(StoreError::OptimisticRequired);
        }
        let mut desired = self.desired.clone();
        edit(&mut desired);
        if desired == self.desired && !self.is_dirty() {
            return Ok(self.revision);
        }
        self.revision += u64::from(desired != self.desired);
        self.desired = desired;
        self.latest_pending = true;
        cx.notify();
        if self.task.is_none() {
            self.start(cx);
        }
        Ok(self.revision)
    }

    fn next(&mut self) -> Option<Candidate<T>> {
        let mut snapshot = self.durable.snapshot().clone();
        let mut completions = Vec::new();
        let revision = if self.publication == Publication::Optimistic {
            let first = self.queue.take();
            if first.is_none() && !self.latest_pending {
                return None;
            }
            self.latest_pending = false;
            snapshot = self.desired.clone();
            if let Some(change) = first {
                completions.push(change.completion);
            }
            while let Some(change) = self.queue.take() {
                completions.push(change.completion);
            }
            self.revision
        } else {
            loop {
                let change = self.queue.take()?;
                if let Some(edit) = change.edit
                    && let Err(error) = edit(&mut snapshot)
                {
                    change.completion.finish(Err(error.clone()));
                    self.error = Some(error);
                    snapshot = self.durable.snapshot().clone();
                    continue;
                }
                completions.push(change.completion);
                break change.revision;
            }
        };
        let candidate = Candidate {
            revision,
            snapshot,
            completions,
        };
        self.candidate = Some(candidate.clone());
        Some(candidate)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let gate = self.gate.clone();
        self.task = Some(cx.spawn(async move |entity, cx| {
            while !gate.is_frozen() {
                let Ok(Some((candidate, writer, writing, unchanged, projection))) =
                    entity.update(cx, |state, cx| {
                        let next = state.next();
                        if state.error.is_some() {
                            cx.notify();
                        }
                        next.map(|candidate| {
                            let unchanged = &candidate.snapshot == state.durable.snapshot();
                            (
                                candidate,
                                state.writer.clone(),
                                state.queue.begin_write(),
                                unchanged,
                                state.projection.clone(),
                            )
                        })
                    })
                else {
                    break;
                };
                let disk_gate = gate.clone();
                let (sender, result) = oneshot::channel();
                cx.background_executor()
                    .spawn(async move {
                        let _writing = writing;
                        let outcome = match disk_gate.normal().await {
                            Some(_guard) => {
                                let outcome = if unchanged {
                                    Ok(())
                                } else {
                                    writer(candidate.snapshot.clone()).await
                                };
                                if outcome.is_ok() {
                                    projection.record(Committed::new(
                                        candidate.snapshot,
                                        candidate.revision,
                                    ));
                                }
                                outcome
                            }
                            None => Err(StoreError::Closing),
                        };
                        let _ = sender.send(outcome);
                    })
                    .detach();
                let outcome = result.await.unwrap_or(Err(StoreError::CompletionDropped));
                if gate.is_frozen() {
                    break;
                }
                if entity
                    .update(cx, |state, cx| state.completed(outcome, cx))
                    .is_err()
                {
                    break;
                }
            }
            let _ = entity.update(cx, |state, _| state.task = None);
        }));
    }

    fn completed(&mut self, outcome: Result<(), StoreError>, cx: &mut Context<Self>) {
        let Some(candidate) = self.candidate.take() else {
            return;
        };
        match &outcome {
            Ok(()) => {
                let previous = std::mem::replace(
                    &mut self.durable,
                    Committed::new(candidate.snapshot.clone(), candidate.revision),
                );
                self.error = None;
                if let Some(hook) = &self.hook {
                    hook(previous.snapshot(), self.durable.snapshot(), cx);
                }
            }
            Err(error) => self.error = Some(error.clone()),
        }
        candidate.finish(outcome);
        cx.notify();
    }

    /// Hydrate schema metadata in all App-facing projections, including the
    /// candidate later published after background completion. No edit is queued.
    pub(crate) fn hydrate(
        &mut self,
        key: std::any::TypeId,
        apply: impl Fn(&mut T) + Send + Sync + 'static,
    ) {
        apply(&mut self.desired);
        self.durable.hydrate(&mut |value| apply(value));
        if let Some(candidate) = &mut self.candidate {
            apply(&mut candidate.snapshot);
        }
        self.projection.hydrate(key, Arc::new(apply));
    }

    /// Memory-only backends publish synchronously, preserving callers' ordering.
    pub(crate) fn update_immediate(
        &mut self,
        edit: impl FnOnce(&mut T),
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        self.update_immediate_checked(
            move |snapshot| {
                edit(snapshot);
                Ok(())
            },
            cx,
        )
    }

    /// Publish a memory-only mutation synchronously; outstanding disk work is rejected.
    pub fn update_immediate_checked(
        &mut self,
        edit: impl FnOnce(&mut T) -> Result<(), StoreError>,
        cx: &mut Context<Self>,
    ) -> Result<(), StoreError> {
        if self.queue.is_closing() {
            return Err(StoreError::Closing);
        }
        if self.publication != Publication::Durable || self.is_dirty() {
            return Err(StoreError::PendingWrites);
        }
        let mut snapshot = self.durable.snapshot().clone();
        edit(&mut snapshot)?;
        if &snapshot != self.durable.snapshot() {
            self.revision += 1;
            self.desired = snapshot.clone();
            let previous =
                std::mem::replace(&mut self.durable, Committed::new(snapshot, self.revision));
            self.projection.record(self.durable.clone());
            self.error = None;
            if let Some(hook) = &self.hook {
                hook(previous.snapshot(), self.durable.snapshot(), cx);
            }
            cx.notify();
        }
        Ok(())
    }

    /// Freeze admission and capture immutable work while the App is still alive.
    pub fn capture(&mut self) -> Captured<T> {
        let dirty = self.is_dirty();
        self.gate.freeze();
        self.queue.close();
        self.task.take();
        self.latest_pending = false;
        let active = self.candidate.take();
        let mut snapshot = match self.publication {
            Publication::Optimistic => self.desired.clone(),
            Publication::Durable => active.as_ref().map_or_else(
                || self.durable.snapshot().clone(),
                |candidate| candidate.snapshot.clone(),
            ),
        };
        let mut completions = Vec::new();
        while let Some(change) = self.queue.take() {
            if let Some(edit) = change.edit {
                let mut next = snapshot.clone();
                if let Err(error) = edit(&mut next) {
                    change.completion.finish(Err(error));
                    continue;
                }
                snapshot = next;
            }
            completions.push(change.completion);
        }
        Captured::new(
            self.durable.clone(),
            active,
            Candidate::new(self.revision, snapshot, completions),
            (self.publication, dirty),
            self.writer.clone(),
            self.gate.clone(),
            self.projection.clone(),
        )
    }

    /// Convenience drain for an owner without dependent captured documents.
    pub fn capture_shutdown(&mut self) -> BoxFuture<'static, Result<(), StoreError>> {
        let work = self.capture();
        Box::pin(async move { work.run().await.into_result() })
    }
}

/// Register this entity's synchronous capture with the explicit coordinator.
pub(crate) fn capture<T: Clone + PartialEq + Send + 'static>(
    entity: &Entity<Persisted<T>>,
    cx: &mut gpui::App,
) -> BoxFuture<'static, Result<(), StoreError>> {
    entity.update(cx, |state, _| state.capture_shutdown())
}

#[cfg(test)]
mod tests;
