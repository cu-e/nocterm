use crate::{batch::Batch, discovery, download, relay, runtime::RuntimeState, upload};
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use nocterm_session::{
    FsError, RemoteFs,
    local_fs::{CancelFlag, LocalDirectory, LocalSource},
};
use nocterm_transfers::{
    TransferCause, TransferEvent, TransferIssue, TransferOperation, TransferPath,
};
use std::{ffi::OsString, sync::Arc, time::Duration};
use tokio::sync::mpsc;

pub(crate) enum FileJob {
    Upload(UploadJob),
    Download(DownloadJob),
    Relay(RelayJob),
}
/// A global backlog permit follows each queued file, including waits for a worker.
pub(crate) struct QueuedJob {
    job: FileJob,
    pending: tokio::sync::OwnedSemaphorePermit,
}
pub(crate) struct RelayJob {
    source_fs: Arc<dyn RemoteFs>,
    source: String,
    remote: String,
    expected: Option<u64>,
}
impl RelayJob {
    pub(crate) fn new(
        source_fs: Arc<dyn RemoteFs>,
        source: String,
        remote: String,
        expected: Option<u64>,
    ) -> Self {
        Self {
            source_fs,
            source,
            remote,
            expected,
        }
    }
    pub(crate) fn source_fs(&self) -> &Arc<dyn RemoteFs> {
        &self.source_fs
    }
    /// The path read on the source filesystem.
    pub(crate) fn source(&self) -> &str {
        &self.source
    }
    /// The path published on the destination filesystem.
    pub(crate) fn remote(&self) -> &str {
        &self.remote
    }
    pub(crate) fn expected(&self) -> Option<u64> {
        self.expected
    }
}
pub(crate) struct UploadJob {
    source: Box<dyn LocalSource>,
    remote: String,
    expected: Option<u64>,
}
impl UploadJob {
    pub(crate) fn new(source: Box<dyn LocalSource>, remote: String, expected: Option<u64>) -> Self {
        Self {
            source,
            remote,
            expected,
        }
    }
    pub(crate) fn into_parts(self) -> (Box<dyn LocalSource>, String, Option<u64>) {
        (self.source, self.remote, self.expected)
    }
}
pub(crate) struct DownloadJob {
    remote: String,
    directory: Arc<dyn LocalDirectory>,
    name: OsString,
    expected: Option<u64>,
}
impl DownloadJob {
    pub(crate) fn new(
        remote: String,
        directory: Arc<dyn LocalDirectory>,
        name: OsString,
        expected: Option<u64>,
    ) -> Self {
        Self {
            remote,
            directory,
            name,
            expected,
        }
    }
    pub(crate) fn remote(&self) -> &str {
        &self.remote
    }
    pub(crate) fn directory(&self) -> &Arc<dyn LocalDirectory> {
        &self.directory
    }
    pub(crate) fn name(&self) -> &std::ffi::OsStr {
        &self.name
    }
    pub(crate) fn expected(&self) -> Option<u64> {
        self.expected
    }
}
pub(crate) enum FileOutcome {
    Completed,
    Skipped(Option<TransferIssue>),
}
#[derive(Clone)]
pub(crate) struct Work {
    state: Arc<RuntimeState>,
    batch: Arc<Batch>,
}
impl Work {
    pub(crate) fn new(state: Arc<RuntimeState>, batch: Arc<Batch>) -> Self {
        Self { state, batch }
    }
    pub(crate) fn state(&self) -> &Arc<RuntimeState> {
        &self.state
    }
    pub(crate) fn batch(&self) -> &Arc<Batch> {
        &self.batch
    }
    pub(crate) fn cancel(&self) -> CancelFlag {
        self.batch.cancellation()
    }
    pub(crate) fn check(&self) -> Result<(), TransferCause> {
        self.cancel().check().map_err(|_| TransferCause::Cancelled)
    }
    pub(crate) async fn remote<T>(
        &self,
        future: BoxFuture<'static, Result<T, FsError>>,
    ) -> Result<T, TransferCause> {
        self.check()?;
        tokio::select! { result=future=>result.map_err(Into::into), ()=self.cancelled()=>Err(TransferCause::Cancelled) }
    }
    pub(crate) async fn queued(&self, job: FileJob) -> Result<QueuedJob, TransferCause> {
        let pending = tokio::select! {
            result = self.state.backlog_slots().clone().acquire_owned() => result.map_err(TransferCause::runtime)?,
            () = self.cancelled() => return Err(TransferCause::Cancelled),
        };
        self.check()?;
        Ok(QueuedJob { job, pending })
    }
    pub(crate) async fn cancelled(&self) {
        while !self.batch.cancelled() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub(crate) fn fail(
        &self,
        path: Option<TransferPath>,
        operation: TransferOperation,
        cause: TransferCause,
    ) {
        if !matches!(cause, TransferCause::Cancelled) {
            self.batch
                .event(TransferEvent::FileFailed(TransferIssue::new(
                    path, operation, cause,
                )));
        }
    }
}
pub(crate) async fn run(state: Arc<RuntimeState>, mut receiver: mpsc::Receiver<Arc<Batch>>) {
    let mut active = FuturesUnordered::new();
    let mut closed = false;
    while !closed || !active.is_empty() {
        tokio::select! {
            batch = receiver.recv(), if !closed && active.len() < 2 => {
                match batch {
                    Some(batch) => active.push(finish(state.clone(), batch)),
                    None => closed = true,
                }
            }
            _ = active.next(), if !active.is_empty() => {},
        }
    }
    state.completed();
}
async fn finish(state: Arc<RuntimeState>, batch: Arc<Batch>) {
    if !batch.cancelled() {
        run_batch(Work::new(state.clone(), batch.clone())).await;
    }
    batch.event(TransferEvent::DiscoveryComplete);
    batch.event(TransferEvent::WorkersDrained);
    state.finish_batch();
}

async fn run_batch(work: Work) {
    work.batch.event(TransferEvent::DiscoveryStarted);
    let (sender, mut receiver) = mpsc::channel(work.state.limits().queue().backlog());
    let discovery_work = work.clone();
    let discovery = tokio::spawn(async move {
        discovery::run(discovery_work, sender).await;
    });
    let mut workers = FuturesUnordered::new();
    let mut closed = false;
    loop {
        if closed && workers.is_empty() {
            break;
        }
        tokio::select! {
            job=receiver.recv(), if !closed&&workers.len()<work.state.limits().queue().parallel()=>{
                if let Some(job)=job {
                    if work.batch.cancelled() {drop(job);} else {workers.push(run_file(work.clone(),job));}
                } else {closed=true;}
            }
            result=workers.next(), if !workers.is_empty()=>{
                if let Some(result)=result {record_outcome(&work,result);}
            }
        }
    }
    if let Err(error) = discovery.await {
        work.fail(
            None,
            TransferOperation::Discover,
            TransferCause::runtime(error),
        );
    }
}
async fn run_file(work: Work, queued: QueuedJob) -> Result<FileOutcome, TransferIssue> {
    let permit = tokio::select! {
        result = work.state.file_slots().clone().acquire_owned() => result.map_err(|error| TransferIssue::new(None, TransferOperation::Runtime, TransferCause::runtime(error)))?,
        () = work.cancelled() => return Err(TransferIssue::new(None, TransferOperation::Runtime, TransferCause::Cancelled)),
    };
    let _permit = permit;
    let QueuedJob { job, pending } = queued;
    drop(pending);
    work.batch.event(TransferEvent::FileStarted);
    match job {
        FileJob::Upload(job) => upload::run(work, job).await,
        FileJob::Download(job) => download::run(work, job).await,
        FileJob::Relay(job) => relay::run(work, job).await,
    }
}
fn record_outcome(work: &Work, outcome: Result<FileOutcome, TransferIssue>) {
    match outcome {
        Ok(FileOutcome::Completed) => work.batch.event(TransferEvent::FileCompleted),
        Ok(FileOutcome::Skipped(issue)) => work.batch.event(TransferEvent::FileSkipped(issue)),
        Err(issue) if matches!(issue.cause(), TransferCause::Cancelled) => {}
        Err(issue) => work.batch.event(TransferEvent::FileFailed(issue)),
    }
}
