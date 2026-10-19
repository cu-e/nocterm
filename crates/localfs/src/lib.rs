//! Native local filesystem capabilities for Explorer and transfer backends.
//!
//! Owns: held-directory traversal, no-follow mutations, bounded listing/statistics,
//! regular source streams and staged private-file publication.
//! Does not own: views, transfer scheduling or remote filesystem protocols.
//! Ports used: none. Ports provided: `LocalFs`, `LocalMutations`.
//! Settings sections: none. Globals: none.
//! Invariants: strict operations pin every Unix ancestor; read-only browsing alone
//! may follow links. Portable traversal rechecks paths and retains its active-ancestor
//! race limitation. Publication errors precede commit; durability warnings follow it.
//! A committed stage relinquishes its temporary name before it can be reused.
mod cursor;
mod directory;
mod listing;
mod mutations;
mod scan;
mod source;
mod stage;
use nocterm_session::{FileMetadata, FsCapabilities, local_fs::*};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct LocalFilesystem {
    base: PathBuf,
}
impl LocalFilesystem {
    pub fn new() -> LocalResult<Self> {
        std::env::current_dir()
            .map(|base| Self { base })
            .map_err(|error| LocalFsError::from_io(".", error))
    }
    pub fn with_base(base: PathBuf) -> LocalResult<Self> {
        if !base.is_absolute() {
            return Err(LocalFsError::InvalidPath(base));
        }
        directory::DirectoryCapability::strict(&base)?;
        Ok(Self { base })
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.base.join(path)
        }
    }
}
impl LocalFs for LocalFilesystem {
    fn home(&self) -> LocalResult<Option<PathBuf>> {
        Ok(directories::UserDirs::new().map(|dirs| dirs.home_dir().to_owned()))
    }
    fn resolve_source(&self, path: &Path) -> LocalResult<PathBuf> {
        let path = self.resolve(path);
        directory::validate_path(&path)?;
        Ok(path)
    }
    fn browse(
        &self,
        path: &Path,
        limits: ListingLimits,
        cancel: &CancelFlag,
    ) -> LocalResult<LocalListing> {
        listing::browse(&self.resolve(path), limits, cancel)
    }
    fn open_directory(&self, path: &Path) -> LocalResult<Arc<dyn LocalDirectory>> {
        Ok(Arc::new(directory::DirectoryCapability::strict(
            &self.resolve(path),
        )?))
    }
    fn open_source(&self, path: &Path) -> LocalResult<Box<dyn LocalSource>> {
        source::open(&self.resolve(path))
    }
    fn scan(
        &self,
        path: &Path,
        policy: &ScanPolicy,
        cancel: &CancelFlag,
        progress: &dyn StatisticsObserver,
    ) -> LocalResult<Statistics> {
        scan::scan(&self.resolve(path), policy, cancel, progress)
    }
}
impl LocalMutations for LocalFilesystem {
    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            metadata: true,
            rename: cfg!(any(
                target_os = "linux",
                target_os = "android",
                target_os = "macos",
                target_os = "ios",
                windows
            )),
            remove: true,
            set_permissions: cfg!(unix),
        }
    }
    fn metadata(&self, path: &Path) -> LocalResult<FileMetadata> {
        mutations::metadata(&self.resolve(path))
    }
    fn rename(&self, path: &Path, name: &OsStr) -> LocalResult<()> {
        mutations::rename(&self.resolve(path), name)
    }
    fn set_permissions(&self, path: &Path, mode: u32) -> LocalResult<()> {
        mutations::permissions(&self.resolve(path), mode)
    }
    fn remove(
        &self,
        path: &Path,
        limits: RemovalLimits,
        cancel: &CancelFlag,
    ) -> LocalResult<RemovalOutcome> {
        mutations::remove(&self.resolve(path), limits, cancel)
    }
}
