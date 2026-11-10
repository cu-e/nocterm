use crate::{batch::Batch, blocking::BlockingDispatcher, scheduler};
use futures::future::BoxFuture;
use nocterm_session::{RemoteFs, local_fs::LocalFs};
use nocterm_transfers::{
    DownloadRequest, Progress, Request, TransferError, TransferId, TransferLimits,
    TransferRequests, TransferService, UploadRequest,
};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
};

pub struct TransferRuntimeDeps {
    local: Arc<dyn LocalFs>,
    limits: TransferLimits,
}
impl TransferRuntimeDeps {
    pub fn new(local: Arc<dyn LocalFs>, limits: TransferLimits) -> Self {
        Self { local, limits }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum TransferRuntimeError {
    #[error("Cannot start the transfer executor: {0}")]
    Executor(#[source] std::io::Error),
}
struct RuntimeOwner {
    runtime: Mutex<Option<Runtime>>,
}
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.get_mut().take() {
            runtime.shutdown_background();
        }
    }
}
struct Admission {
    sender: Option<mpsc::Sender<Arc<Batch>>>,
    sequence: u64,
}
pub(crate) struct RuntimeState {
    batches: Mutex<BTreeMap<TransferId, Arc<Batch>>>,
    revision: Arc<crate::changes::Changes>,
    local: Arc<dyn LocalFs>,
    limits: TransferLimits,
    blocking: BlockingDispatcher,
    file_slots: Arc<tokio::sync::Semaphore>,
    backlog_slots: Arc<tokio::sync::Semaphore>,
    completion: Completion,
}
struct Completion {
    done: AtomicBool,
    changed: Notify,
}
impl RuntimeState {
    pub(crate) fn local(&self) -> &Arc<dyn LocalFs> {
        &self.local
    }
    pub(crate) fn limits(&self) -> TransferLimits {
        self.limits
    }
    pub(crate) fn blocking(&self) -> &BlockingDispatcher {
        &self.blocking
    }
    pub(crate) fn file_slots(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.file_slots
    }
    pub(crate) fn backlog_slots(&self) -> &Arc<tokio::sync::Semaphore> {
        &self.backlog_slots
    }
    pub(crate) fn completed(&self) {
        self.completion.done.store(true, Ordering::Release);
        self.completion.changed.notify_waiters();
    }
    async fn drain(&self) {
        loop {
            let notified = self.completion.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.completion.done.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub(crate) fn finish_batch(&self) {
        self.trim(&mut self.batches.lock());
    }
    fn trim(&self, batches: &mut BTreeMap<TransferId, Arc<Batch>>) {
        let mut finished: Vec<_> = batches
            .iter()
            .filter(|(_, batch)| batch.finished())
            .map(|(id, batch)| (*id, batch.sequence()))
            .collect();
        finished.sort_by_key(|(_, sequence)| *sequence);
        let excess = finished.len().saturating_sub(self.limits.queue().history());
        for (id, _) in finished.into_iter().take(excess) {
            batches.remove(&id);
            self.revision.advance();
        }
    }
}
/// This handle owns no UI state and needs no global lookup.
pub struct TransferRuntime {
    owner: Arc<RuntimeOwner>,
    admission: Mutex<Admission>,
    state: Arc<RuntimeState>,
}
impl TransferRuntime {
    pub fn start(deps: TransferRuntimeDeps) -> Result<Arc<Self>, TransferRuntimeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(TransferRuntimeError::Executor)?;
        let (sender, receiver) = mpsc::channel(deps.limits.queue().queued());
        let state = Arc::new(RuntimeState {
            batches: Mutex::new(BTreeMap::new()),
            revision: Arc::default(),
            local: deps.local,
            limits: deps.limits,
            blocking: BlockingDispatcher::new(deps.limits.queue().parallel()),
            file_slots: Arc::new(tokio::sync::Semaphore::new(deps.limits.queue().parallel())),
            backlog_slots: Arc::new(tokio::sync::Semaphore::new(deps.limits.queue().backlog())),
            completion: Completion {
                done: AtomicBool::new(false),
                changed: Notify::new(),
            },
        });
        runtime.spawn(scheduler::run(state.clone(), receiver));
        Ok(Arc::new(Self {
            owner: Arc::new(RuntimeOwner {
                runtime: Mutex::new(Some(runtime)),
            }),
            admission: Mutex::new(Admission {
                sender: Some(sender),
                sequence: 0,
            }),
            state,
        }))
    }
    fn admit(&self, request: Request) -> Result<TransferId, TransferError> {
        request.validate(self.state.limits.discovery())?;
        let mut admission = self.admission.lock();
        if admission.sender.is_none() {
            return Err(TransferError::ShuttingDown);
        }
        let sequence = admission
            .sequence
            .checked_add(1)
            .ok_or(TransferError::SequenceExhausted)?;
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(TransferError::runtime)?;
        let id = TransferId::from_bytes(bytes)?;
        let batch = Arc::new(Batch::new(
            sequence,
            id,
            request,
            self.state.limits.stream(),
            self.state.revision.clone(),
        ));
        let mut batches = self.state.batches.lock();
        if batches.contains_key(&id) {
            return Err(TransferError::InvalidId);
        }
        admission
            .sender
            .as_ref()
            .ok_or(TransferError::ShuttingDown)?
            .try_send(batch.clone())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => TransferError::Full,
                mpsc::error::TrySendError::Closed(_) => TransferError::ShuttingDown,
            })?;
        admission.sequence = sequence;
        batches.insert(id, batch);
        self.state.revision.advance();
        self.state.trim(&mut batches);
        Ok(id)
    }
    fn cancel_batch(&self, id: TransferId) -> Result<(), TransferError> {
        let batches = self.state.batches.lock();
        let batch = batches.get(&id).ok_or(TransferError::Missing)?;
        batch.cancel();
        Ok(())
    }
    fn freeze(&self) {
        let mut admission = self.admission.lock();
        admission.sender.take();
        for batch in self.state.batches.lock().values() {
            batch.cancel();
        }
    }
}
impl TransferService for TransferRuntime {
    fn enqueue(&self, request: UploadRequest) -> Result<TransferId, TransferError> {
        self.admit(Request::Upload(request))
    }
    fn enqueue_download(&self, request: DownloadRequest) -> Result<TransferId, TransferError> {
        self.admit(Request::Download(request))
    }
    fn snapshot(&self) -> Vec<Progress> {
        let batches = self.state.batches.lock();
        let mut snapshots: Vec<_> = batches
            .values()
            .map(|batch| (batch.sequence(), batch.snapshot()))
            .collect();
        snapshots.sort_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
        snapshots
            .into_iter()
            .map(|(_, progress)| progress)
            .collect()
    }
    fn revision(&self) -> Option<u64> {
        Some(self.state.revision.revision())
    }
    fn changed_since(&self, revision: u64) -> Option<BoxFuture<'static, ()>> {
        let changes = self.state.revision.clone();
        Some(Box::pin(async move { changes.since(revision).await }))
    }
    fn cancel(&self, id: TransferId) -> Result<(), TransferError> {
        self.cancel_batch(id)
    }
    fn retry(&self, id: TransferId, fs: Arc<dyn RemoteFs>) -> Result<TransferId, TransferError> {
        let request = {
            let batches = self.state.batches.lock();
            let batch = batches.get(&id).ok_or(TransferError::Missing)?;
            if !batch.finished() {
                return Err(TransferError::NotRetryable);
            }
            batch.request().clone().with_fs(fs)
        };
        self.admit(request)
    }
    fn freeze_and_drain(&self) -> BoxFuture<'static, Result<(), TransferError>> {
        self.freeze();
        let state = self.state.clone();
        let owner = self.owner.clone();
        Box::pin(async move {
            state.drain().await;
            drop(owner);
            Ok(())
        })
    }
}
impl TransferRequests for TransferRuntime {
    fn submit(&self, request: Request) -> Result<TransferId, TransferError> {
        self.admit(request)
    }
    fn progress(&self, id: TransferId) -> Result<Progress, TransferError> {
        self.state
            .batches
            .lock()
            .get(&id)
            .map(|batch| batch.snapshot())
            .ok_or(TransferError::Missing)
    }
    fn cancel(&self, id: TransferId) -> Result<(), TransferError> {
        self.cancel_batch(id)
    }
}
impl Drop for TransferRuntime {
    fn drop(&mut self) {
        self.freeze();
    }
}
