//! Exclusive private-file creation, staged atomic publication and bounded reads.
//!
//! These helpers replace or open the final path without following its symlink. Parent
//! directories must be trusted: this is not a pinned-directory traversal API.

mod bounded;
mod private;

pub use bounded::{BoundedReadError, read_regular_bounded};
pub use private::{PrivateFileError, open_private_regular, require_private_regular};

use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write as _},
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

/// Creates a new write-only file, with mode 0600 on Unix.
///
/// Existing paths, including dangling symlinks, are refused. This does not create
/// parent directories. The returned file belongs to the caller's stream lifetime.
pub fn create_private(path: &Path) -> Result<File, FileError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| FileError::at(path, source))
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

/// A completed namespace replacement whose parent directory still needs best-effort sync.
/// Dropping this receipt cannot remove or roll back the published file.
#[must_use = "finish parent sync outside the owner's publication lock"]
pub struct CommittedPublication {
    destination: PathBuf,
}

impl CommittedPublication {
    pub fn sync_parent(self) {
        sync_parent_best_effort(&self.destination);
    }
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

    fn stage(
        path: &Path,
        bytes: &[u8],
        permissions: Option<Permissions>,
    ) -> Result<Self, FileError> {
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
        temporary
            .write_all(bytes)
            .map_err(|source| FileError::at(path, source))?;
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
        self.commit()?.sync_parent();
        Ok(())
    }

    /// Commits only the namespace replacement. The caller can publish matching
    /// memory under its authority lock, then sync the parent after releasing it.
    pub fn commit(self) -> Result<CommittedPublication, FileError> {
        self.temporary
            .persist(&self.destination)
            .map_err(|error| FileError::at(&self.destination, error.error))?;
        Ok(CommittedPublication {
            destination: self.destination,
        })
    }

    /// Publishes only if the final path does not exist, including a dangling link.
    /// Returns the actual published descriptor; no destination is overwritten.
    ///
    /// Success commits the namespace operation. The tempfile fallback can leave
    /// a second staging hard link, and parent sync is best effort. A private-key
    /// owner must validate this descriptor and confirm held-parent durability
    /// before announcing readiness. A postcommit refusal must retain the identity,
    /// rather than deleting or silently regenerating the destination.
    pub fn publish_new(self) -> Result<File, FileError> {
        let file = self
            .temporary
            .persist_noclobber(&self.destination)
            .map_err(|error| FileError::at(&self.destination, error.error))?;
        sync_parent_best_effort(&self.destination);
        Ok(file)
    }
}

fn sync_parent_best_effort(path: &Path) {
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent_directory(path)) {
        let _ = directory.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests;
