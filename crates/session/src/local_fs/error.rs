use std::{io, path::PathBuf, sync::Arc};

pub type LocalResult<T> = Result<T, LocalFsError>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum LocalFsError {
    #[error("{}: invalid path", .0.display())]
    InvalidPath(PathBuf),
    #[error("invalid native filename")]
    InvalidName,
    #[error("{}: no such file or directory", .0.display())]
    NotFound(PathBuf),
    #[error("{}: permission denied", .0.display())]
    PermissionDenied(PathBuf),
    #[error("{}: already exists", .0.display())]
    AlreadyExists(PathBuf),
    #[error("{}: symbolic link or reparse point refused", .0.display())]
    LinkRefused(PathBuf),
    #[error("{}: not a regular file", .0.display())]
    NotRegular(PathBuf),
    #[error("unsupported local filesystem operation: {0}")]
    Unsupported(&'static str),
    #[error("operation cancelled")]
    Cancelled,
    #[error("{0} bound exceeded; the result is partial")]
    BoundExceeded(&'static str),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
    #[error("{removed} entries deleted before failure: {source}")]
    PartialRemoval {
        removed: usize,
        #[source]
        source: Box<LocalFsError>,
    },
}
impl LocalFsError {
    pub fn from_io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        let path = path.into();
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            _ => Self::Io {
                path,
                source: Arc::new(source),
            },
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the file was published, but directory durability could not be confirmed: {source}")]
pub struct DurabilityWarning {
    #[source]
    source: Arc<io::Error>,
}
impl DurabilityWarning {
    pub fn new(source: io::Error) -> Self {
        Self {
            source: Arc::new(source),
        }
    }
    pub fn kind(&self) -> io::ErrorKind {
        self.source.kind()
    }
}
#[derive(Clone, Debug)]
pub enum PublicationOutcome {
    Published,
    PublishedWithWarning(DurabilityWarning),
}
