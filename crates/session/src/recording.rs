//! Bounded immutable append requests; adapters own files and services schedule work.
use futures::future::BoxFuture;
use nocterm_core::SharedError;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_APPEND_BYTES: usize = 8192;

#[derive(Clone, Debug)]
pub struct RecordingOptions {
    pub directory: Option<PathBuf>,
    pub max_file_mib: u32,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("output recording is unavailable")]
    Unavailable,
    #[error("output recording is closing")]
    Closing,
    #[error("output recording queue is full")]
    Full,
    #[error("output recording size limit reached")]
    Limit,
    #[error("append block must contain 1..={MAX_APPEND_BYTES} bytes")]
    InvalidBlock,
    #[error("append sequence or offset does not match the physical prefix")]
    SequenceMismatch,
    #[error("output recording completion was dropped")]
    CompletionDropped,
    #[error("could not create {}: {source}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: SharedError<io::Error>,
    },
    #[error("could not append {} at {offset}: {source}", path.display())]
    Append {
        path: PathBuf,
        offset: u64,
        #[source]
        source: SharedError<io::Error>,
    },
    #[error("could not flush {}: {source}", path.display())]
    Flush {
        path: PathBuf,
        #[source]
        source: SharedError<io::Error>,
    },
    #[error("could not sync {}: {source}", path.display())]
    Sync {
        path: PathBuf,
        #[source]
        source: SharedError<io::Error>,
    },
}

impl RecordingError {
    pub fn create(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Create {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
    pub fn append(path: impl Into<PathBuf>, offset: u64, source: io::Error) -> Self {
        Self::Append {
            path: path.into(),
            offset,
            source: SharedError::from_error(source),
        }
    }
    pub fn flush(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Flush {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
    pub fn sync(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Sync {
            path: path.into(),
            source: SharedError::from_error(source),
        }
    }
}

impl PartialEq for RecordingError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unavailable, Self::Unavailable)
            | (Self::Closing, Self::Closing)
            | (Self::Full, Self::Full)
            | (Self::Limit, Self::Limit)
            | (Self::InvalidBlock, Self::InvalidBlock)
            | (Self::SequenceMismatch, Self::SequenceMismatch)
            | (Self::CompletionDropped, Self::CompletionDropped) => true,
            (Self::Create { path: p1, .. }, Self::Create { path: p2, .. }) => p1 == p2,
            (
                Self::Append {
                    path: p1,
                    offset: o1,
                    ..
                },
                Self::Append {
                    path: p2,
                    offset: o2,
                    ..
                },
            ) => p1 == p2 && o1 == o2,
            (Self::Flush { path: p1, .. }, Self::Flush { path: p2, .. }) => p1 == p2,
            (Self::Sync { path: p1, .. }, Self::Sync { path: p2, .. }) => p1 == p2,
            _ => false,
        }
    }
}

impl Eq for RecordingError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendBlock {
    sequence: u64,
    expected_offset: u64,
    bytes: Arc<[u8]>,
}
impl AppendBlock {
    /// Sequence numbering starts at one. Bytes may be decoded UTF-8 fragments.
    pub fn new(
        sequence: u64,
        expected_offset: u64,
        bytes: Arc<[u8]>,
    ) -> Result<Self, RecordingError> {
        if bytes.is_empty() || bytes.len() > MAX_APPEND_BYTES {
            return Err(RecordingError::InvalidBlock);
        }
        if sequence == 0 {
            return Err(RecordingError::SequenceMismatch);
        }
        Ok(Self {
            sequence,
            expected_offset,
            bytes,
        })
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn expected_offset(&self) -> u64 {
        self.expected_offset
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Written bytes include the last incomplete block; durability requires `synced`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysicalPrefix {
    complete_sequence: u64,
    written_bytes: u64,
    partial_bytes: u64,
    synced: bool,
    closed: bool,
}
impl PhysicalPrefix {
    pub fn new(
        complete_sequence: u64,
        written_bytes: u64,
        partial_bytes: u64,
        synced: bool,
        closed: bool,
    ) -> Self {
        Self {
            complete_sequence,
            written_bytes,
            partial_bytes,
            synced,
            closed,
        }
    }
    pub fn complete_sequence(&self) -> u64 {
        self.complete_sequence
    }
    pub fn written_bytes(&self) -> u64 {
        self.written_bytes
    }
    pub fn partial_bytes(&self) -> u64 {
        self.partial_bytes
    }
    pub fn synced(&self) -> bool {
        self.synced
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
}

#[derive(Clone, Debug)]
pub struct AppendOutcome {
    prefix: PhysicalPrefix,
    error: Option<RecordingError>,
}
impl AppendOutcome {
    pub fn new(prefix: PhysicalPrefix, error: Option<RecordingError>) -> Self {
        Self { prefix, error }
    }
    pub fn prefix(&self) -> &PhysicalPrefix {
        &self.prefix
    }
    pub fn error(&self) -> Option<&RecordingError> {
        self.error.as_ref()
    }
}
/// Seal preserves an incomplete physical prefix even when flush/sync fails.
pub type SealOutcome = AppendOutcome;

pub trait OutputLog: Send + Sync {
    fn path(&self) -> &Path;
    /// Futures own their operation, and native work begins when polled by a worker.
    fn append(&self, block: AppendBlock) -> BoxFuture<'static, AppendOutcome>;
    fn seal(&self) -> BoxFuture<'static, SealOutcome>;
}
pub trait OutputLogFactory: Send + Sync {
    fn create(
        &self,
        options: RecordingOptions,
    ) -> BoxFuture<'static, Result<Arc<dyn OutputLog>, RecordingError>>;
}
