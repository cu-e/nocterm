use super::{Gate, lock};
use crate::{
    DirEntry, EntryKind, FileMetadata, FsCapabilities, FsError, FsFuture, RemoteFs,
    fs::{RemoteDownload, RemoteUpload, UploadMode},
};
use std::sync::{Arc, Mutex};
mod control;
mod native;
mod streams;
mod tree;
use control::Control;
pub use control::{ListingReplyError, PendingListing};
use tree::Tree;

/// Request boundaries available for one-shot failure and suspension injection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsOperation {
    Home,
    List,
    Stat,
    Metadata,
    CreateDir,
    Rename,
    RemoveFile,
    RemoveDir,
    Permissions,
    Upload,
    Write,
    Finish,
    Cancel,
    Download,
    Read,
    Close,
    Sync,
    DirectorySync,
}

/// Observations through the fake's public surface; handle counts include pending
/// writes/reads until their last future is released.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FsActivity {
    pub requests: usize,
    pub live_uploads: usize,
    pub live_downloads: usize,
    pub peak_downloads: usize,
    pub completed_uploads: usize,
    pub canceled_uploads: usize,
    pub abandoned_uploads: usize,
    pub closed_downloads: usize,
    pub max_read_request: usize,
    pub peak_uploads: usize,
    pub max_write_chunk: usize,
    pub max_write_batch: usize,
    pub acknowledged_bytes: usize,
}

struct Shared {
    tree: Mutex<Tree>,
    home: Mutex<String>,
    control: Mutex<Control>,
    capabilities: Mutex<FsCapabilities>,
    limits: Mutex<Limits>,
    activity: Mutex<FsActivity>,
    native_control: Mutex<native::Controls>,
    scripted_reads: Mutex<std::collections::VecDeque<(String, Vec<u8>)>>,
}
struct Limits {
    chunk: usize,
    file: usize,
}
impl Shared {
    async fn before(&self, operation: FsOperation, path: &str) -> Result<(), FsError> {
        {
            let mut activity = lock(&self.activity);
            activity.requests = activity.requests.saturating_add(1);
        }
        let (gate, error) = lock(&self.control).take(operation, path);
        if let Some(gate) = gate {
            gate.wait().await;
        }
        error.map_or(Ok(()), Err)
    }

    fn supports(&self, enabled: bool, operation: &'static str) -> Result<(), FsError> {
        if enabled {
            Ok(())
        } else {
            Err(FsError::Unsupported(operation.into()))
        }
    }
}

/// A coherent path tree and staged streams, with no real I/O or executor. All
/// ancestors are real directories; links are listed/unlinked but never traversed.
#[derive(Clone)]
pub struct InMemoryFs {
    shared: Arc<Shared>,
}
impl Default for InMemoryFs {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared {
                tree: Mutex::new(Tree::default()),
                home: Mutex::new("/".into()),
                control: Mutex::default(),
                capabilities: Mutex::new(FsCapabilities {
                    metadata: true,
                    rename: true,
                    remove: true,
                    set_permissions: true,
                }),
                limits: Mutex::new(Limits {
                    chunk: 32 * 1024,
                    file: 8 * 1024 * 1024,
                }),
                activity: Mutex::default(),
                native_control: Mutex::default(),
                scripted_reads: Mutex::default(),
            }),
        }
    }
}
impl InMemoryFs {
    /// Replace listing responses with controllable protocol replies. Other
    /// operations still use the coherent tree; requests queue only when polled.
    pub fn script_listings(&self) {
        lock(&self.shared.control).script_listings();
    }

    pub fn has_pending_listings(&self) -> bool {
        lock(&self.shared.control).has_pending_listings()
    }

    pub fn take_pending_listing(&self) -> Option<PendingListing> {
        lock(&self.shared.control).take_pending_listing()
    }

    pub fn set_capabilities(&self, capabilities: FsCapabilities) {
        *lock(&self.shared.capabilities) = capabilities;
    }

    /// Chunk reads are additionally bounded by the caller; writes above this
    /// limit fail before changing staged bytes. Files have a separate size bound.
    pub fn set_limits(&self, chunk: usize, file: usize) -> Result<(), FsError> {
        if chunk == 0 || chunk > 32 * 1024 || file == 0 {
            return Err(FsError::Unsupported("invalid fake stream limits".into()));
        }
        *lock(&self.shared.limits) = Limits { chunk, file };
        Ok(())
    }

    pub fn set_home(&self, path: &str) -> Result<(), FsError> {
        if !path.starts_with('/')
            || path.contains('\0')
            || path.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(FsError::Unsupported("invalid fake home".into()));
        }
        if lock(&self.shared.tree).metadata(path)?.kind != EntryKind::Directory {
            return Err(FsError::NotFound { path: path.into() });
        }
        *lock(&self.shared.home) = path.into();
        Ok(())
    }
    /// One bounded, deliberately hostile protocol reply for consumer tests.
    pub fn override_next_read(&self, path: &str, bytes: Vec<u8>) -> Result<(), FsError> {
        let mut reads = lock(&self.shared.scripted_reads);
        if reads.len() >= 64 || bytes.len() > 64 * 1024 {
            return Err(FsError::Unsupported("scripted read response bound".into()));
        }
        reads.push_back((path.into(), bytes));
        Ok(())
    }
    pub fn declare_size(&self, path: &str, size: Option<u64>) -> Result<(), FsError> {
        lock(&self.shared.tree).declare_size(path, size)
    }
    pub fn seed_file(&self, path: &str, bytes: impl Into<Vec<u8>>) -> Result<(), FsError> {
        let bytes = bytes.into();
        if bytes.len() > lock(&self.shared.limits).file {
            return Err(FsError::Unsupported("fake file size bound".into()));
        }
        lock(&self.shared.tree).seed(path, Some(bytes), None)
    }

    pub fn seed_symlink(&self, path: &str, target_kind: EntryKind) -> Result<(), FsError> {
        lock(&self.shared.tree).seed(path, None, Some(target_kind))
    }
    pub fn seed_special(&self, path: &str) -> Result<(), FsError> {
        lock(&self.shared.tree).seed(path, None, None)
    }
    pub fn bytes(&self, path: &str) -> Result<Vec<u8>, FsError> {
        lock(&self.shared.tree).bytes(path)
    }
    pub fn activity(&self) -> FsActivity {
        lock(&self.shared.activity).clone()
    }

    pub fn suspend_next(&self, operation: FsOperation, path: impl Into<String>, gate: Gate) {
        lock(&self.shared.control).add(operation, path.into(), Some(gate), None);
    }
    pub fn fail_next(&self, operation: FsOperation, path: impl Into<String>, error: FsError) {
        lock(&self.shared.control).add(operation, path.into(), None, Some(error));
    }

    fn request<T: Send + 'static>(
        &self,
        operation: FsOperation,
        path: &str,
        apply: impl FnOnce(&Shared, &str) -> Result<T, FsError> + Send + 'static,
    ) -> FsFuture<T> {
        let shared = self.shared.clone();
        let path = path.to_owned();
        Box::pin(async move {
            shared.before(operation, &path).await?;
            apply(&shared, &path)
        })
    }
}

impl RemoteFs for InMemoryFs {
    fn capabilities(&self) -> FsCapabilities {
        *lock(&self.shared.capabilities)
    }
    fn home(&self) -> FsFuture<String> {
        self.request(FsOperation::Home, "/", |shared, _| {
            Ok(lock(&shared.home).clone())
        })
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let shared = self.shared.clone();
        let path = path.to_owned();
        Box::pin(async move {
            shared.before(FsOperation::List, &path).await?;
            let response = lock(&shared.control).listing(&path)?;
            if let Some(response) = response {
                return response.await.unwrap_or(Err(FsError::NotConnected));
            }
            lock(&shared.tree).list(&path)
        })
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        self.request(FsOperation::Stat, path, |shared, path| {
            match lock(&shared.tree).metadata(path) {
                Ok(meta) => Ok(Some(tree::entry(path, meta))),
                Err(FsError::NotFound { .. }) => Ok(None),
                Err(error) => Err(error),
            }
        })
    }
    fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
        self.request(FsOperation::Metadata, path, |shared, path| {
            shared.supports(
                lock(&shared.capabilities).metadata,
                "detailed file metadata",
            )?;
            lock(&shared.tree).metadata(path)
        })
    }
    fn create_dir(&self, path: &str) -> FsFuture<()> {
        self.request(FsOperation::CreateDir, path, |shared, path| {
            lock(&shared.tree).create_dir(path)
        })
    }
    fn rename(&self, old: &str, new: &str) -> FsFuture<()> {
        let new = new.to_owned();
        self.request(FsOperation::Rename, old, move |shared, old| {
            shared.supports(lock(&shared.capabilities).rename, "file rename")?;
            lock(&shared.tree).rename(old, &new)
        })
    }
    fn remove_file(&self, path: &str) -> FsFuture<()> {
        self.request(FsOperation::RemoveFile, path, |shared, path| {
            shared.supports(lock(&shared.capabilities).remove, "file removal")?;
            lock(&shared.tree).remove(path, false)
        })
    }
    fn remove_dir(&self, path: &str) -> FsFuture<()> {
        self.request(FsOperation::RemoveDir, path, |shared, path| {
            shared.supports(lock(&shared.capabilities).remove, "directory removal")?;
            lock(&shared.tree).remove(path, true)
        })
    }
    fn set_permissions(&self, path: &str, permissions: u32) -> FsFuture<()> {
        self.request(FsOperation::Permissions, path, move |shared, path| {
            shared.supports(
                lock(&shared.capabilities).set_permissions,
                "file permissions",
            )?;
            lock(&shared.tree).permissions(path, permissions)
        })
    }
    fn upload(&self, path: &str, mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        let shared = self.shared.clone();
        let path = path.to_owned();
        Box::pin(async move {
            shared.before(FsOperation::Upload, &path).await?;
            lock(&shared.tree).upload_path(&path, mode)?;
            Ok(Box::new(streams::Upload::new(shared, path, mode)) as Box<dyn RemoteUpload>)
        })
    }

    fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        let shared = self.shared.clone();
        let path = path.to_owned();
        Box::pin(async move {
            shared.before(FsOperation::Download, &path).await?;
            let bytes = lock(&shared.tree).bytes(&path)?;
            Ok(Box::new(streams::Reader::new(shared, path, bytes)) as Box<dyn RemoteDownload>)
        })
    }
}

/// A synchronous operation observation hook. Tests may coordinate an adapter's
/// blocking worker here; the fake itself never creates or blocks an executor.
pub trait LocalRequestObserver: Send + Sync {
    fn request(&self, operation: FsOperation, path: &std::path::Path);
}
impl<F: Fn(FsOperation, &std::path::Path) + Send + Sync> LocalRequestObserver for F {
    fn request(&self, operation: FsOperation, path: &std::path::Path) {
        self(operation, path);
    }
}
impl InMemoryFs {
    pub fn seed_native_file(
        &self,
        path: &std::path::Path,
        bytes: impl Into<Vec<u8>>,
    ) -> crate::local_fs::LocalResult<()> {
        let path = native::resolve(path)?;
        let bytes = bytes.into();
        if bytes.len() > lock(&self.shared.limits).file {
            return Err(crate::local_fs::LocalFsError::BoundExceeded(
                "fake file size",
            ));
        }
        lock(&self.shared.tree)
            .seed_native(&path, bytes)
            .map_err(|error| native::map(error, &path))
    }
    pub fn seed_native_directory(
        &self,
        path: &std::path::Path,
    ) -> crate::local_fs::LocalResult<()> {
        let path = native::resolve(path)?;
        lock(&self.shared.tree)
            .directory_native(&path)
            .map_err(|error| native::map(error, &path))
    }
    pub fn seed_native_alias(
        &self,
        path: &std::path::Path,
        target: &std::path::Path,
    ) -> crate::local_fs::LocalResult<()> {
        let path = native::resolve(path)?;
        let target = native::resolve(target)?;
        lock(&self.shared.tree)
            .seed_native_alias(&path, &target)
            .map_err(|error| native::map(error, &path))
    }
    pub fn fail_next_local(
        &self,
        operation: FsOperation,
        path: std::path::PathBuf,
        error: crate::local_fs::LocalFsError,
    ) {
        lock(&self.shared.native_control).fail(operation, path, error);
    }
    pub fn observe_local(&self, observer: std::sync::Arc<dyn LocalRequestObserver>) {
        lock(&self.shared.native_control).observe(observer);
    }
}
impl crate::local_fs::LocalFs for InMemoryFs {
    fn home(&self) -> crate::local_fs::LocalResult<Option<std::path::PathBuf>> {
        Ok(Some(native::base()))
    }
    fn resolve_source(
        &self,
        path: &std::path::Path,
    ) -> crate::local_fs::LocalResult<std::path::PathBuf> {
        native::resolve(path)
    }
    fn browse(
        &self,
        path: &std::path::Path,
        limits: crate::local_fs::ListingLimits,
        cancel: &crate::local_fs::CancelFlag,
    ) -> crate::local_fs::LocalResult<crate::local_fs::LocalListing> {
        native::browse(self.shared.clone(), path, limits, cancel)
    }
    fn open_directory(
        &self,
        path: &std::path::Path,
    ) -> crate::local_fs::LocalResult<std::sync::Arc<dyn crate::local_fs::LocalDirectory>> {
        native::directory(self.shared.clone(), path)
    }
    fn open_source(
        &self,
        path: &std::path::Path,
    ) -> crate::local_fs::LocalResult<Box<dyn crate::local_fs::LocalSource>> {
        native::source(self.shared.clone(), path)
    }
    fn scan(
        &self,
        path: &std::path::Path,
        policy: &crate::local_fs::ScanPolicy,
        cancel: &crate::local_fs::CancelFlag,
        progress: &dyn crate::local_fs::StatisticsObserver,
    ) -> crate::local_fs::LocalResult<crate::local_fs::Statistics> {
        native::scan(self.shared.clone(), path, policy, cancel, progress)
    }
}
impl crate::local_fs::LocalMutations for InMemoryFs {
    fn capabilities(&self) -> FsCapabilities {
        native::capabilities(&self.shared)
    }
    fn metadata(&self, path: &std::path::Path) -> crate::local_fs::LocalResult<FileMetadata> {
        native::metadata(&self.shared, path)
    }
    fn rename(
        &self,
        path: &std::path::Path,
        name: &std::ffi::OsStr,
    ) -> crate::local_fs::LocalResult<()> {
        native::rename(&self.shared, path, name)
    }
    fn set_permissions(
        &self,
        path: &std::path::Path,
        mode: u32,
    ) -> crate::local_fs::LocalResult<()> {
        native::permissions(&self.shared, path, mode)
    }
    fn remove(
        &self,
        path: &std::path::Path,
        limits: crate::local_fs::RemovalLimits,
        cancel: &crate::local_fs::CancelFlag,
    ) -> crate::local_fs::LocalResult<crate::local_fs::RemovalOutcome> {
        native::remove(self.shared.clone(), path, limits, cancel)
    }
}
