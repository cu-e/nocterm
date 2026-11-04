//! Private native snapshots own their lock and durable publication phase.
//! Codecs and source bindings remain with their callers; ancestors are trusted.
use nocterm_core::{
    SharedCause,
    fs::{
        AtomicPublication, FileError, PrivateFileError, create_private, open_private_regular,
        require_private_regular,
    },
};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum SnapshotError {
    #[error("the private snapshot directory is unavailable")]
    Directory(#[source] io::Error),
    #[error("the private snapshot lock is unavailable")]
    Lock(#[source] io::Error),
    #[error("the private snapshot file is unsafe")]
    Private(#[source] PrivateFileError),
    #[error("the private snapshot could not be published")]
    Publication(#[source] FileError),
    #[error("the snapshot was published but durability is unconfirmed")]
    Stored(#[source] SharedCause),
}

pub(super) struct PrivateSnapshot {
    path: PathBuf,
    parent: File,
    _lock: File,
    published: bool,
}
impl PrivateSnapshot {
    pub(super) fn acquire(root: &Path, lock: &str, name: &str) -> Result<Self, SnapshotError> {
        nocterm_core::paths::ensure_private_dir(root).map_err(SnapshotError::Directory)?;
        let parent = File::open(root).map_err(SnapshotError::Directory)?;
        let path = root.join(lock);
        let lock = match create_private(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                open_private_regular(&path).map_err(SnapshotError::Private)?
            }
            Err(error) => return Err(SnapshotError::Publication(error)),
        };
        require_private_regular(&lock).map_err(SnapshotError::Private)?;
        lock.try_lock()
            .map_err(|error| SnapshotError::Lock(io::Error::other(error)))?;
        Ok(Self {
            path: root.join(name),
            parent,
            _lock: lock,
            published: false,
        })
    }
    pub(super) fn open(&self) -> Result<File, PrivateFileError> {
        open_private_regular(&self.path)
    }
    /// Identity creation always uses no-clobber, even after a previous attempt.
    pub(super) fn publish_new(&mut self, bytes: &[u8]) -> Result<File, SnapshotError> {
        let file = AtomicPublication::stage_private(&self.path, bytes)
            .map_err(SnapshotError::Publication)?
            .publish_new()
            .map_err(SnapshotError::Publication)?;
        self.published = true;
        Ok(file)
    }
    pub(super) fn save(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let staged = AtomicPublication::stage_private(&self.path, bytes)
            .map_err(SnapshotError::Publication)?;
        let file = if self.published {
            let old = self.open().map_err(SnapshotError::Private)?;
            require_private_regular(&old).map_err(SnapshotError::Private)?;
            staged.publish().map_err(SnapshotError::Publication)?;
            self.open()
                .map_err(|error| SnapshotError::Stored(SharedCause::new(error)))?
        } else {
            let file = staged.publish_new().map_err(SnapshotError::Publication)?;
            // A subsequent failure never restores create-new permission.
            self.published = true;
            file
        };
        self.confirm(&file)
    }
    pub(super) fn accept_read(&mut self, file: &File) -> Result<(), SnapshotError> {
        self.confirm(file)?;
        self.published = true;
        Ok(())
    }
    pub(super) fn confirm(&self, file: &File) -> Result<(), SnapshotError> {
        require_private_regular(file)
            .map_err(|error| SnapshotError::Stored(SharedCause::new(error)))?;
        file.sync_all()
            .map_err(|error| SnapshotError::Stored(SharedCause::new(error)))?;
        self.parent
            .sync_all()
            .map_err(|error| SnapshotError::Stored(SharedCause::new(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_require_exclusive_ownership_and_explicit_acceptance_before_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut snapshot = PrivateSnapshot::acquire(root, "state.lock", "state").unwrap();
        snapshot.save(b"first").unwrap();
        assert!(PrivateSnapshot::acquire(root, "state.lock", "state").is_err());
        assert!(snapshot.publish_new(b"identity replacement").is_err());
        snapshot.save(b"second").unwrap();
        drop(snapshot);
        let mut reopened = PrivateSnapshot::acquire(root, "state.lock", "state").unwrap();
        assert!(reopened.save(b"unaccepted").is_err());
        reopened.accept_read(&reopened.open().unwrap()).unwrap();
        reopened.save(b"accepted").unwrap();
        assert_eq!(std::fs::read(root.join("state")).unwrap(), b"accepted");
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_replacement_preserves_both_the_snapshot_and_a_link_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut snapshot = PrivateSnapshot::acquire(root, "state.lock", "state").unwrap();
        snapshot.save(b"original").unwrap();
        std::fs::set_permissions(root.join("state"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(snapshot.save(b"unsafe").is_err());
        std::fs::rename(root.join("state"), root.join("target")).unwrap();
        symlink(root.join("target"), root.join("state")).unwrap();
        assert!(snapshot.save(b"linked").is_err());
        assert_eq!(std::fs::read(root.join("target")).unwrap(), b"original");
    }
}
