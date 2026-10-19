//! Exclusive private-file creation and staged atomic publication.
//!
//! These helpers replace the final path without following its symlink. Parent
//! directories must be trusted: this is not a pinned-directory traversal API.

mod admitted_anonymous;
pub use admitted_anonymous::AdmittedAnonymousError;
mod admitted_path;
mod admitted_publication;
#[cfg(unix)]
pub use admitted_publication::publication_native_scratch_bytes;
pub use admitted_publication::{
    AdmittedFileError, AdmittedPublication, AdmittedStageError, DiagnosticCredits,
    PreparedPrivateStage, io_error_custom_allocation_bytes,
};
mod directory_policy;
pub use directory_policy::{
    DirectoryMessagePlan, PrivateDirectoryCheckError, PrivateDirectoryReason,
};
mod current_directory;
#[cfg(unix)]
mod environment_buffer;
pub use admitted_path::{
    AdmittedPath, LeafPathPlan, PathAdmissionError, PreparedPathJoin, WidePathPlan,
};
pub use current_directory::{CurrentDirectoryBuffer, CurrentDirectoryError, DirectoryObservation};
#[cfg(unix)]
pub use environment_buffer::{EnvironmentBuffer, EnvironmentObservation};

use std::{
    fs::{self, File, OpenOptions, Permissions},
    io,
    path::{Path, PathBuf},
};

/// A filesystem operation failed before publication committed.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

impl FileError {
    fn at(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    /// The original operating-system error kind, for recovery decisions.
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::Io { source, .. } => source.kind(),
        }
    }

    /// Preserve this error as the source when adapting an existing I/O API.
    pub fn into_io_error(self) -> io::Error {
        io::Error::new(self.kind(), self)
    }
}

/// A staged producer failed, or the native stage could not be prepared/synced.
/// No destination was published in either case; concrete sources remain typed.
#[derive(Debug, thiserror::Error)]
pub enum StageError<E> {
    #[error(transparent)]
    Native(#[from] FileError),
    #[error("snapshot producer failed: {0}")]
    Producer(#[source] E),
}

/// Creates a new write-only file, with mode 0600 on Unix.
///
/// Existing paths, including dangling symlinks, are refused. This does not create
/// parent directories. The returned file belongs to the caller's stream lifetime.
pub fn create_private(path: &Path) -> Result<File, FileError> {
    let options = private_open_options(false);
    options
        .open(path)
        .map_err(|source| FileError::at(path, source))
}

fn private_open_options(read: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(read).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

/// Publishes bytes at their exact path, creating missing parent directories.
///
/// The final symlink is replaced, never followed. Bytes and file metadata are
/// synced before rename; directory sync after commit is best effort. Success
/// means publication committed, rather than guaranteeing power-loss durability.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let directory = parent_directory(path);
    fs::create_dir_all(directory).map_err(|source| FileError::at(directory, source))?;
    AtomicPublication::stage_private(path, bytes)?.publish()
}

/// A synced snapshot awaiting publication; dropping it removes the staging file.
///
/// Owners can keep their lock while staging, check revisions or cancellation
/// after I/O, and publish only if those checks pass. No background work is owned.
#[must_use = "publish the staged snapshot or drop it to discard the write"]
pub struct AtomicPublication {
    temporary: tempfile::NamedTempFile,
    destination: PathBuf,
}

impl AtomicPublication {
    /// Stages a mode-0600 snapshot in an existing parent directory.
    pub fn stage_private(path: &Path, bytes: &[u8]) -> Result<Self, FileError> {
        Self::stage(path, bytes, None)
    }

    /// Stages bytes with the owner's explicitly selected final permissions.
    ///
    /// For metadata-preserving files such as SSH trust records. The temporary
    /// file is created exclusively with private permissions before applying the
    /// requested mode. Ownership and access-control lists are not copied.
    pub fn stage_with_permissions(
        path: &Path,
        bytes: &[u8],
        permissions: Permissions,
    ) -> Result<Self, FileError> {
        Self::stage(path, bytes, Some(permissions))
    }

    /// Streams a producer into the same private stage as byte snapshots.
    ///
    /// The producer may reject its complete candidate before returning success.
    /// Only success is synced; its typed error drops the partially written stage.
    /// Parent directories must exist. Publication remains a separate owner action.
    pub fn stage_private_with<E: std::error::Error + Send + Sync + 'static>(
        path: &Path,
        produce: impl FnOnce(&mut dyn io::Write) -> Result<(), E>,
    ) -> Result<Self, StageError<E>> {
        Self::stage_producer(path, None, produce)
    }

    fn stage(
        path: &Path,
        bytes: &[u8],
        permissions: Option<Permissions>,
    ) -> Result<Self, FileError> {
        Self::stage_producer(path, permissions, |writer| {
            writer
                .write_all(bytes)
                .map_err(|source| FileError::at(path, source))
        })
        .map_err(|error| match error {
            StageError::Native(error) | StageError::Producer(error) => error,
        })
    }

    fn stage_producer<E: std::error::Error + Send + Sync + 'static>(
        path: &Path,
        permissions: Option<Permissions>,
        produce: impl FnOnce(&mut dyn io::Write) -> Result<(), E>,
    ) -> Result<Self, StageError<E>> {
        let mut temporary = tempfile::NamedTempFile::new_in(parent_directory(path))
            .map_err(|source| FileError::at(path, source))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temporary
                .as_file()
                .set_permissions(permissions.unwrap_or_else(|| Permissions::from_mode(0o600)))
                .map_err(|source| FileError::at(path, source))?;
        }
        #[cfg(not(unix))]
        if let Some(permissions) = permissions {
            temporary
                .as_file()
                .set_permissions(permissions)
                .map_err(|source| FileError::at(path, source))?;
        }
        produce(&mut temporary).map_err(StageError::Producer)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| FileError::at(path, source))?;
        Ok(Self {
            temporary,
            destination: path.to_owned(),
        })
    }

    /// Atomically replaces the final path without following its symlink.
    ///
    /// No error is returned after rename: callers may now publish matching
    /// in-memory state. Best-effort directory sync cannot turn a commit into an
    /// apparent failed write. Ancestors are not pinned against concurrent changes.
    pub fn publish(self) -> Result<(), FileError> {
        publish_temporary(self.temporary, &self.destination)
            .map_err(|source| FileError::at(&self.destination, source))
    }
}

fn publish_temporary(temporary: tempfile::NamedTempFile, destination: &Path) -> io::Result<()> {
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent_directory(destination)) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests;
