use crate::{LimitKind, Request, StreamLimits, TransferId};
use nocterm_session::{
    FsError, Target,
    local_fs::{DurabilityWarning, LocalFsError},
};
use std::{error::Error, fmt, path::PathBuf, sync::Arc};
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
    pub id: TransferId,
    pub target: Target,
    pub destination: String,
    pub direction: TransferDirection,
    pub state: TransferState,
    pub discovered_files: u64,
    pub completed_files: u64,
    /// Destination directory preparations acknowledged by the adapter; an
    /// already-existing directory can also be prepared. These are not files.
    pub prepared_directories: u64,
    pub skipped_files: u64,
    pub failed_files: u64,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub discovery_complete: bool,
    pub errors: Vec<TransferIssue>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferPath {
    Local(PathBuf),
    Remote(String),
}
impl fmt::Display for TransferPath {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => write!(output, "{}", path.display()),
            Self::Remote(path) => output.write_str(path),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOperation {
    Discover,
    Open,
    Read,
    Write,
    Publish,
    Cleanup,
    Runtime,
}
impl fmt::Display for TransferOperation {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::Discover => "discover",
            Self::Open => "open",
            Self::Read => "read",
            Self::Write => "write",
            Self::Publish => "publish",
            Self::Cleanup => "cleanup",
            Self::Runtime => "runtime",
        })
    }
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum TransferCause {
    #[error(transparent)]
    Local(LocalFsError),
    #[error(transparent)]
    Remote(#[from] FsError),
    #[error("Transfer exceeds its {0} limit.")]
    Limit(LimitKind),
    #[error("File size changed (expected {expected:?}, transferred {observed}).")]
    SizeChanged {
        expected: Option<u64>,
        observed: u64,
    },
    #[error("Reader returned {received} bytes for a {requested}-byte chunk request.")]
    OversizedChunk { requested: usize, received: usize },
    #[error("Filename cannot be represented safely at the destination.")]
    InvalidName,
    #[error("Skipped a link or special file.")]
    SpecialFile,
    #[error("Transfer cancelled.")]
    Cancelled,
    #[error("File was published, but directory durability could not be confirmed: {0}")]
    Durability(#[source] DurabilityWarning),
    #[error("Transfer runtime failed: {0}")]
    Runtime(#[source] Arc<dyn Error + Send + Sync>),
}
impl From<LocalFsError> for TransferCause {
    fn from(error: LocalFsError) -> Self {
        match error {
            LocalFsError::Cancelled => Self::Cancelled,
            error => Self::Local(error),
        }
    }
}

#[derive(Debug)]
struct IssueDetails {
    path: Option<TransferPath>,
    operation: TransferOperation,
    cause: TransferCause,
}
/// Immutable notice identity is retained by snapshot clones, so consumers can
/// detect newly appended diagnostics without flattening typed causes to strings.
#[derive(Clone, Debug)]
pub struct TransferIssue(Arc<IssueDetails>);
impl TransferIssue {
    pub fn new(
        path: Option<TransferPath>,
        operation: TransferOperation,
        cause: TransferCause,
    ) -> Self {
        Self(Arc::new(IssueDetails {
            path,
            operation,
            cause,
        }))
    }
    pub fn path(&self) -> Option<&TransferPath> {
        self.0.path.as_ref()
    }
    pub fn operation(&self) -> TransferOperation {
        self.0.operation
    }
    pub fn cause(&self) -> &TransferCause {
        &self.0.cause
    }
}
impl PartialEq for TransferIssue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for TransferIssue {}
impl fmt::Display for TransferIssue {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = self.path() {
            write!(output, "{path}: ")?;
        }
        write!(output, "{}: {}", self.operation(), self.cause())
    }
}
impl Error for TransferIssue {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause())
    }
}
pub enum TransferEvent {
    DiscoveryStarted,
    FileStarted,
    DiscoveredFile {
        bytes: u64,
    },
    Acknowledged {
        bytes: u64,
    },
    FileCompleted,
    /// A destination directory is ready after its successful create/open call.
    DirectoryPrepared,
    FileSkipped(Option<TransferIssue>),
    FileFailed(TransferIssue),
    Notice(TransferIssue),
    DiscoveryComplete,
    WorkersDrained,
    CancelRequested,
}
pub struct ProgressTracker {
    progress: Progress,
    workers_done: bool,
    detail_limit: usize,
    cancel_requested: bool,
}
impl ProgressTracker {
    pub fn new(id: TransferId, request: &Request, limits: StreamLimits) -> Self {
        Self {
            progress: Progress {
                id,
                target: request.target().clone(),
                destination: request.destination(),
                direction: request.direction(),
                state: TransferState::Queued,
                discovered_files: 0,
                completed_files: 0,
                prepared_directories: 0,
                skipped_files: 0,
                failed_files: 0,
                total_bytes: 0,
                sent_bytes: 0,
                discovery_complete: false,
                errors: Vec::new(),
            },
            workers_done: false,
            detail_limit: limits.details(),
            cancel_requested: false,
        }
    }
    pub fn snapshot(&self) -> Progress {
        self.progress.clone()
    }
    pub fn apply(&mut self, event: TransferEvent) {
        if self.progress.state.finished() {
            return;
        }
        match event {
            TransferEvent::DiscoveryStarted if !self.progress.state.finished() => {
                self.progress.state = TransferState::Discovering
            }
            TransferEvent::FileStarted if !self.progress.state.finished() => {
                self.progress.state = TransferState::Running
            }
            TransferEvent::DiscoveredFile { bytes } => {
                self.progress.discovered_files = self.progress.discovered_files.saturating_add(1);
                self.progress.total_bytes = self.progress.total_bytes.saturating_add(bytes);
            }
            TransferEvent::Acknowledged { bytes } => {
                self.progress.sent_bytes = self.progress.sent_bytes.saturating_add(bytes)
            }
            TransferEvent::FileCompleted => {
                self.progress.completed_files = self.progress.completed_files.saturating_add(1)
            }
            TransferEvent::DirectoryPrepared => {
                self.progress.prepared_directories =
                    self.progress.prepared_directories.saturating_add(1)
            }
            TransferEvent::FileSkipped(issue) => {
                self.progress.skipped_files = self.progress.skipped_files.saturating_add(1);
                if let Some(issue) = issue {
                    self.notice(issue);
                }
            }
            TransferEvent::FileFailed(issue) => {
                self.progress.failed_files = self.progress.failed_files.saturating_add(1);
                self.notice(issue);
            }
            TransferEvent::Notice(issue) => self.notice(issue),
            TransferEvent::DiscoveryComplete => self.progress.discovery_complete = true,
            TransferEvent::WorkersDrained => self.workers_done = true,
            TransferEvent::CancelRequested => self.cancel_requested = true,
            TransferEvent::DiscoveryStarted | TransferEvent::FileStarted => {}
        }
        if self.workers_done && self.progress.discovery_complete {
            self.progress.state = if self.cancel_requested {
                TransferState::Cancelled
            } else if self.progress.failed_files > 0 {
                TransferState::Failed
            } else {
                TransferState::Completed
            };
        }
    }
    fn notice(&mut self, issue: TransferIssue) {
        if self.progress.errors.len() < self.detail_limit {
            self.progress.errors.push(issue);
        }
    }
}
