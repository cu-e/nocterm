//! Streaming batches pin a filesystem and destination when queued. Discovery,
//! reads, progress and publication never run on the UI thread.

mod download;

use nocterm_session::{
    FsError, RemoteFs, Target,
    fs::{UploadMode, path},
};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    runtime::{Builder, Runtime},
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use uuid::Uuid;

const QUEUED_BATCHES: usize = 8;
const FILE_BACKLOG: usize = 32;
const PARALLEL_FILES: usize = 4;
const CHUNK_SIZE: usize = 32 * 1024;
const MAX_ERROR_DETAILS: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionPolicy {
    #[default]
    Skip,
    Rename,
    Replace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferState {
    Queued,
    Discovering,
    Running,
    Completed,
    Cancelled,
    Failed,
}
impl TransferState {
    pub fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub id: Uuid,
    pub target: Target,
    pub destination: String,
    pub direction: TransferDirection,
    pub state: TransferState,
    pub discovered_files: u64,
    pub completed_files: u64,
    pub skipped_files: u64,
    pub failed_files: u64,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub discovery_complete: bool,
    pub errors: Vec<String>,
}
#[derive(Clone)]
pub struct UploadRequest {
    pub sources: Vec<PathBuf>,
    pub target: Target,
    pub destination: String,
    pub fs: Arc<dyn RemoteFs>,
    pub collisions: CollisionPolicy,
}
#[derive(Clone)]
pub struct DownloadRequest {
    pub sources: Vec<String>,
    pub target: Target,
    pub local_destination: PathBuf,
    pub fs: Arc<dyn RemoteFs>,
    pub collisions: CollisionPolicy,
}
#[derive(Clone)]
enum Request {
    Upload(UploadRequest),
    Download(DownloadRequest),
}
impl Request {
    fn target(&self) -> &Target {
        match self {
            Self::Upload(r) => &r.target,
            Self::Download(r) => &r.target,
        }
    }
    fn destination(&self) -> String {
        match self {
            Self::Upload(r) => r.destination.clone(),
            Self::Download(r) => r.local_destination.display().to_string(),
        }
    }
    fn direction(&self) -> TransferDirection {
        match self {
            Self::Upload(_) => TransferDirection::Upload,
            Self::Download(_) => TransferDirection::Download,
        }
    }
    fn fs(&self) -> &Arc<dyn RemoteFs> {
        match self {
            Self::Upload(r) => &r.fs,
            Self::Download(r) => &r.fs,
        }
    }
    fn set_fs(&mut self, fs: Arc<dyn RemoteFs>) {
        match self {
            Self::Upload(r) => r.fs = fs,
            Self::Download(r) => r.fs = fs,
        }
    }
    fn collisions(&self) -> CollisionPolicy {
        match self {
            Self::Upload(r) => r.collisions,
            Self::Download(r) => r.collisions,
        }
    }
}
struct Batch {
    request: Request,
    progress: Mutex<Progress>,
    cancel: AtomicBool,
    destinations: Arc<Mutex<HashSet<String>>>,
    local_destinations: Arc<Mutex<HashSet<PathBuf>>>,
}
impl Batch {
    fn new(request: Request) -> Self {
        Self {
            progress: Mutex::new(Progress {
                id: Uuid::new_v4(),
                target: request.target().clone(),
                destination: request.destination(),
                direction: request.direction(),
                state: TransferState::Queued,
                discovered_files: 0,
                completed_files: 0,
                skipped_files: 0,
                failed_files: 0,
                total_bytes: 0,
                sent_bytes: 0,
                discovery_complete: false,
                errors: Vec::new(),
            }),
            request,
            cancel: AtomicBool::new(false),
            destinations: Arc::default(),
            local_destinations: Arc::default(),
        }
    }
    fn error(&self, error: impl ToString) {
        let mut p = self.progress.lock();
        p.failed_files += 1;
        if p.errors.len() < MAX_ERROR_DETAILS {
            p.errors.push(error.to_string());
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("The transfer queue is full. Wait for a batch to finish.")]
    Full,
    #[error("No files were selected.")]
    Empty,
    #[error("Invalid transfer destination.")]
    InvalidDestination,
    #[error("The transfer no longer exists.")]
    Missing,
}

/// Retained by the application, not a panel. Up to four files stream globally.
pub struct Transfers {
    runtime: Option<Runtime>,
    sender: mpsc::Sender<Arc<Batch>>,
    batches: Arc<Mutex<BTreeMap<Uuid, Arc<Batch>>>>,
}
impl Transfers {
    pub fn new() -> std::io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nocterm-transfers")
            .enable_all()
            .build()?;
        let (sender, mut receiver) = mpsc::channel::<Arc<Batch>>(QUEUED_BATCHES);
        let slots = Arc::new(Semaphore::new(PARALLEL_FILES));
        runtime.spawn(async move {
            // One batch discovers at a time. Files within it are concurrent;
            // subsequent batches remain bounded and visible as queued.
            while let Some(batch) = receiver.recv().await {
                run(batch, slots.clone()).await;
            }
        });
        Ok(Self {
            runtime: Some(runtime),
            sender,
            batches: Arc::default(),
        })
    }
    pub fn enqueue(&self, request: UploadRequest) -> Result<Uuid, QueueError> {
        if request.sources.is_empty() {
            return Err(QueueError::Empty);
        }
        if !request.destination.starts_with('/') || request.destination.contains('\0') {
            return Err(QueueError::InvalidDestination);
        }
        self.enqueue_request(Request::Upload(request))
    }
    pub fn enqueue_download(&self, request: DownloadRequest) -> Result<Uuid, QueueError> {
        if request.sources.is_empty() {
            return Err(QueueError::Empty);
        }
        if !request.local_destination.is_absolute()
            || request.sources.iter().any(|p| !download::valid_source(p))
        {
            return Err(QueueError::InvalidDestination);
        }
        self.enqueue_request(Request::Download(request))
    }
    fn enqueue_request(&self, request: Request) -> Result<Uuid, QueueError> {
        let batch = Arc::new(Batch::new(request));
        let id = batch.progress.lock().id;
        self.sender
            .try_send(batch.clone())
            .map_err(|_| QueueError::Full)?;
        let mut batches = self.batches.lock();
        // Retain bounded history, without dropping running jobs.
        if batches.len() >= 64 {
            let old = batches
                .iter()
                .find(|(_, b)| b.progress.lock().state.finished())
                .map(|(id, _)| *id);
            if let Some(id) = old {
                batches.remove(&id);
            }
        }
        batches.insert(id, batch);
        Ok(id)
    }
    pub fn snapshot(&self) -> Vec<Progress> {
        self.batches
            .lock()
            .values()
            .map(|b| b.progress.lock().clone())
            .collect()
    }
    pub fn cancel(&self, id: Uuid) {
        if let Some(batch) = self.batches.lock().get(&id) {
            batch.cancel.store(true, Ordering::Release);
        }
    }
    /// Retry is an explicit user action. The caller supplies the confirmed
    /// current filesystem of the same target after a possible reconnect.
    pub fn retry(&self, id: Uuid, fs: Arc<dyn RemoteFs>) -> Result<Uuid, QueueError> {
        let mut request = self
            .batches
            .lock()
            .get(&id)
            .ok_or(QueueError::Missing)?
            .request
            .clone();
        request.set_fs(fs);
        self.enqueue_request(request)
    }
}
impl Drop for Transfers {
    fn drop(&mut self) {
        for batch in self.batches.lock().values() {
            batch.cancel.store(true, Ordering::Release);
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

enum Job {
    Upload(FileJob),
    Download(download::FileJob),
}
struct FileJob {
    local: PathBuf,
    remote: String,
}
async fn run(batch: Arc<Batch>, slots: Arc<Semaphore>) {
    if batch.cancelled() {
        batch.progress.lock().state = TransferState::Cancelled;
        return;
    }
    batch.progress.lock().state = TransferState::Discovering;
    let (send, mut receive) = mpsc::channel(FILE_BACKLOG);
    let discover_batch = batch.clone();
    let mut discovery = tokio::spawn(async move {
        match &discover_batch.request {
            Request::Upload(_) => discover(discover_batch, send).await,
            Request::Download(_) => download::discover(discover_batch, send).await,
        }
    });
    let mut workers = JoinSet::new();
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)), if !batch.cancelled() => {},
            _ = tokio::time::sleep(Duration::ZERO), if batch.cancelled() => {
                discovery.abort(); workers.abort_all();
                while workers.join_next().await.is_some() {}
                let _ = (&mut discovery).await;
                batch.progress.lock().state = TransferState::Cancelled; return;
            },
            Some(result) = workers.join_next(), if !workers.is_empty() => {
                if let Err(error) = result { batch.error(error); }
            },
            job = receive.recv(), if workers.len() < PARALLEL_FILES => {
                let Some(job) = job else { break; };
                batch.progress.lock().state = TransferState::Running;
                let batch = batch.clone(); let slots = slots.clone();
                workers.spawn(async move {
                    let _slot = slots.acquire().await.map_err(|e| FsError::Other(e.to_string()))?;
                    let result = match job { Job::Upload(job) => upload_file(&batch, job).await, Job::Download(job) => download::download_file(&batch, job).await };
                    match result {
                        Ok(true) => batch.progress.lock().completed_files += 1,
                        Ok(false) => batch.progress.lock().skipped_files += 1,
                        Err(error) => batch.error(error),
                    }
                    Ok::<_, FsError>(())
                });
            }
        }
    }
    if let Err(error) = (&mut discovery).await {
        batch.error(format!("transfer discovery stopped: {error}"));
    }
    while !workers.is_empty() {
        tokio::select! {
            result = workers.join_next() => { if let Some(Err(e)) = result { batch.error(e); } },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if batch.cancelled() {
                    workers.abort_all();
                    while workers.join_next().await.is_some() {}
                    break;
                }
            }
        }
    }
    let mut progress = batch.progress.lock();
    progress.state = if batch.cancelled() {
        TransferState::Cancelled
    } else if progress.failed_files > 0 {
        TransferState::Failed
    } else {
        TransferState::Completed
    };
}

/// Depth-first directory iterators keep discovery memory proportional to depth,
/// not total file count. The queue applies back-pressure before further reads.
async fn discover(batch: Arc<Batch>, send: mpsc::Sender<Job>) {
    let Request::Upload(request) = &batch.request else {
        return;
    };
    let mut seen = HashSet::new();
    for source in &request.sources {
        if batch.cancelled() {
            break;
        }
        let Some(name) = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|s| *s != "." && *s != ".." && !s.contains('/'))
        else {
            batch.error(format!(
                "{}: filename cannot be represented remotely",
                source.display()
            ));
            continue;
        };
        // Duplicated selections must not race their own destination.
        if !seen.insert(source.clone()) {
            continue;
        }
        let remote = path::join(&request.destination, name);
        if let Err(e) = discover_one(&batch, &send, source.clone(), remote).await {
            batch.error(e);
        }
    }
    batch.progress.lock().discovery_complete = true;
}
async fn discover_one(
    batch: &Batch,
    send: &mpsc::Sender<Job>,
    local: PathBuf,
    remote: String,
) -> Result<(), FsError> {
    let mut current = Some((local, remote));
    let mut stack: Vec<(tokio::fs::ReadDir, String)> = Vec::new();
    loop {
        if batch.cancelled() {
            return Ok(());
        }
        if let Some((local, remote)) = current.take() {
            let metadata = tokio::fs::symlink_metadata(&local)
                .await
                .map_err(|e| local_error(&local, e))?;
            if metadata.is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
                let mut p = batch.progress.lock();
                p.skipped_files += 1;
                if p.errors.len() < MAX_ERROR_DETAILS {
                    p.errors
                        .push(format!("{}: skipped link or special file", local.display()));
                }
            } else if metadata.is_file() {
                {
                    let mut p = batch.progress.lock();
                    p.discovered_files += 1;
                    p.total_bytes = p.total_bytes.saturating_add(metadata.len());
                }
                send.send(Job::Upload(FileJob { local, remote }))
                    .await
                    .map_err(|_| FsError::Other("upload discovery cancelled".into()))?;
            } else {
                batch.request.fs().create_dir(&remote).await?;
                stack.push((
                    tokio::fs::read_dir(&local)
                        .await
                        .map_err(|e| local_error(&local, e))?,
                    remote,
                ));
            }
        }
        let Some((directory, parent)) = stack.last_mut() else {
            break;
        };
        match directory
            .next_entry()
            .await
            .map_err(|e| FsError::Other(e.to_string()))?
        {
            Some(entry) => {
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| FsError::Other("non-UTF8 filename cannot be uploaded".into()))?;
                current = Some((entry.path(), path::join(parent, &name)));
            }
            None => {
                stack.pop();
            }
        }
    }
    Ok(())
}
async fn upload_file(batch: &Batch, job: FileJob) -> Result<bool, FsError> {
    let fs = batch.request.fs();
    let Some((destination, _reservation)) = reserve_destination(batch, &job.remote).await? else {
        return Ok(false);
    };
    // Opening a local symlink that replaced a discovered file is rejected.
    let metadata = tokio::fs::symlink_metadata(&job.local)
        .await
        .map_err(|e| local_error(&job.local, e))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(FsError::Other(format!(
            "{} changed before upload",
            job.local.display()
        )));
    }
    #[cfg(unix)]
    let mut file = tokio::fs::File::from_std(download::open_local_source(&job.local)?);
    #[cfg(not(unix))]
    let mut file = {
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
        options
            .open(&job.local)
            .await
            .map_err(|e| local_error(&job.local, e))?
    };
    let opened = file
        .metadata()
        .await
        .map_err(|e| local_error(&job.local, e))?;
    if !opened.is_file() {
        return Err(FsError::Other(
            "local source is no longer a regular file".into(),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        if opened.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(FsError::Other(
                "local reparse points are not uploaded".into(),
            ));
        }
    }
    let mode = if batch.request.collisions() == CollisionPolicy::Replace {
        UploadMode::Replace
    } else {
        UploadMode::Create
    };
    let mut writer = match fs.upload(&destination, mode).await {
        Err(FsError::AlreadyExists { .. })
            if batch.request.collisions() == CollisionPolicy::Skip =>
        {
            return Ok(false);
        }
        result => result?,
    };
    let result = async {
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut published = 0;
        loop {
            let mut chunks = Vec::with_capacity(8);
            for _ in 0..8 {
                if batch.cancelled() {
                    return Err(FsError::Other("upload cancelled".into()));
                }
                let count = file
                    .read(&mut buffer)
                    .await
                    .map_err(|e| local_error(&job.local, e))?;
                if count == 0 {
                    break;
                }
                chunks.push(buffer[..count].to_vec());
            }
            if chunks.is_empty() {
                break;
            }
            let count = chunks.iter().map(Vec::len).sum::<usize>();
            writer.write_batch(chunks).await?;
            published += count as u64;
            if published > opened.len() {
                return Err(FsError::Other("local file grew during upload".into()));
            }
            let mut progress = batch.progress.lock();
            progress.sent_bytes = progress.sent_bytes.saturating_add(count as u64);
        }
        if published != opened.len()
            || file
                .metadata()
                .await
                .map_err(|e| local_error(&job.local, e))?
                .len()
                != opened.len()
        {
            return Err(FsError::Other(
                "local file size changed during upload".into(),
            ));
        }
        if batch.cancelled() {
            return Err(FsError::Other("upload cancelled".into()));
        }
        Ok::<_, FsError>(published)
    }
    .await;
    match result {
        Ok(_) => match writer.finish().await {
            Err(FsError::AlreadyExists { .. })
                if batch.request.collisions() == CollisionPolicy::Skip =>
            {
                Ok(false)
            }
            result => result.map(|_| true),
        },
        Err(error) => {
            let _ = writer.cancel().await;
            Err(error)
        }
    }
}
struct DestinationReservation {
    path: String,
    active: Arc<Mutex<HashSet<String>>>,
}
impl Drop for DestinationReservation {
    fn drop(&mut self) {
        self.active.lock().remove(&self.path);
    }
}
async fn reserve_destination(
    batch: &Batch,
    original: &str,
) -> Result<Option<(String, DestinationReservation)>, FsError> {
    let mut number = 0;
    loop {
        if batch.cancelled() {
            return Err(FsError::Other("upload cancelled".into()));
        }
        let candidate = if number == 0 {
            original.to_owned()
        } else {
            renamed(original, number)
        };
        let exists = batch.request.fs().stat(&candidate).await?.is_some();
        if exists && batch.request.collisions() == CollisionPolicy::Skip {
            return Ok(None);
        }
        if (!exists || batch.request.collisions() == CollisionPolicy::Replace)
            && batch.destinations.lock().insert(candidate.clone())
        {
            return Ok(Some((
                candidate.clone(),
                DestinationReservation {
                    path: candidate,
                    active: batch.destinations.clone(),
                },
            )));
        }
        match batch.request.collisions() {
            CollisionPolicy::Skip => return Ok(None),
            CollisionPolicy::Rename => {
                number += 1;
                if number > 10_000 {
                    return Err(FsError::Other(
                        "too many destination name collisions".into(),
                    ));
                }
            }
            CollisionPolicy::Replace => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
}
fn local_error(path: &Path, error: std::io::Error) -> FsError {
    FsError::Other(format!("{}: {error}", path.display()))
}
fn renamed(path: &str, number: u32) -> String {
    let name = path::file_name(path);
    let (stem, extension) = name
        .rsplit_once('.')
        .filter(|(s, _)| !s.is_empty())
        .map_or((name, String::new()), |(s, e)| (s, format!(".{e}")));
    path::join(path::parent(path), &format!("{stem} ({number}){extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_files_keep_their_extension_and_directory() {
        assert_eq!(renamed("/a/report.tar.gz", 2), "/a/report.tar (2).gz");
        assert_eq!(renamed("/.env", 1), "/.env (1)");
    }
}
