use super::{FsOperation, Shared, lock};
use crate::{FileMetadata, FsCapabilities, FsError, fs::UploadMode, local_fs::*};
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) type Observer = Arc<dyn super::LocalRequestObserver>;
#[derive(Default)]
pub(super) struct Controls {
    failures: VecDeque<(FsOperation, PathBuf, LocalFsError)>,
    observer: Option<Observer>,
}
impl Controls {
    pub(super) fn fail(&mut self, operation: FsOperation, path: PathBuf, error: LocalFsError) {
        self.failures.push_back((operation, path, error));
    }
    pub(super) fn observe(&mut self, observer: Observer) {
        self.observer = Some(observer);
    }
    fn take(
        &mut self,
        operation: FsOperation,
        path: &Path,
    ) -> (Option<Observer>, Option<LocalFsError>) {
        let error = self
            .failures
            .iter()
            .position(|(op, key, _)| *op == operation && key == path)
            .and_then(|index| self.failures.remove(index))
            .map(|(_, _, error)| error);
        (self.observer.clone(), error)
    }
}
pub(super) fn before(shared: &Shared, operation: FsOperation, path: &Path) -> LocalResult<()> {
    let (observer, error) = lock(&shared.native_control).take(operation, path);
    if let Some(observer) = observer {
        observer.request(operation, path);
    }
    error.map_or(Ok(()), Err)
}
pub(super) fn map(error: FsError, path: &Path) -> LocalFsError {
    match error {
        FsError::NotConnected => LocalFsError::NotFound(path.to_owned()),
        FsError::PermissionDenied { .. } => LocalFsError::PermissionDenied(path.to_owned()),
        FsError::NotFound { .. } => LocalFsError::NotFound(path.to_owned()),
        FsError::AlreadyExists { .. } => LocalFsError::AlreadyExists(path.to_owned()),
        FsError::Unsupported(_) => LocalFsError::InvalidPath(path.to_owned()),
        FsError::Other(_) => LocalFsError::Unsupported("unclassified fake remote error"),
    }
}
pub(super) fn base() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from("C:\\")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/")
    }
}
pub(super) fn resolve(path: &Path) -> LocalResult<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        base().join(path)
    };
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
        || path.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err(LocalFsError::InvalidPath(path));
    }
    Ok(path)
}
pub(super) fn directory(shared: Arc<Shared>, path: &Path) -> LocalResult<Arc<dyn LocalDirectory>> {
    let path = resolve(path)?;
    before(&shared, FsOperation::Metadata, &path)?;
    let id = lock(&shared.tree)
        .directory_id(&path)
        .map_err(|error| map(error, &path))?;
    Ok(Arc::new(Directory { shared, id, path }))
}
struct Directory {
    shared: Arc<Shared>,
    id: u64,
    path: PathBuf,
}
impl LocalDirectory for Directory {
    fn location(&self) -> PathBuf {
        self.path.clone()
    }
    fn identity(&self) -> DirectoryIdentity {
        DirectoryIdentity::native(Arc::as_ptr(&self.shared).addr() as u64, self.id)
    }
    fn entries(&self) -> LocalResult<Box<dyn LocalCursor>> {
        before(&self.shared, FsOperation::List, &self.path)?;
        let names = lock(&self.shared.tree)
            .held_list(self.id)
            .map_err(|error| map(error, &self.path))?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        Ok(Box::new(Cursor { names }))
    }
    fn metadata(&self, name: &OsStr) -> LocalResult<Option<FileMetadata>> {
        check_name(name)?;
        before(&self.shared, FsOperation::Metadata, &self.path.join(name))?;
        lock(&self.shared.tree)
            .held_metadata(self.id, name)
            .map_err(|error| map(error, &self.path.join(name)))
    }
    fn child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>> {
        self.descend(name, false)
    }
    fn create_child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>> {
        self.descend(name, true)
    }
    fn open_source(&self, name: &OsStr) -> LocalResult<Box<dyn LocalSource>> {
        check_name(name)?;
        let path = self.path.join(name);
        before(&self.shared, FsOperation::Download, &path)?;
        let (metadata, bytes) = {
            let tree = lock(&self.shared.tree);
            let metadata = tree
                .held_metadata(self.id, name)
                .map_err(|error| map(error, &path))?
                .ok_or_else(|| LocalFsError::NotFound(path.clone()))?;
            if metadata.is_symlink {
                return Err(LocalFsError::LinkRefused(path));
            }
            if metadata.kind != crate::EntryKind::File {
                return Err(LocalFsError::NotRegular(path));
            }
            let bytes = tree
                .held_bytes(self.id, name)
                .map_err(|error| map(error, &path))?;
            (metadata, bytes)
        };
        opened_source(self.shared.clone(), path, metadata, bytes)
    }
    fn stage(&self) -> LocalResult<Box<dyn LocalStage>> {
        before(&self.shared, FsOperation::Upload, &self.path)?;
        let mut activity = lock(&self.shared.activity);
        activity.live_uploads = activity.live_uploads.saturating_add(1);
        drop(activity);
        Ok(Box::new(Stage {
            shared: self.shared.clone(),
            id: self.id,
            path: self.path.clone(),
            bytes: Vec::new(),
            synced: false,
            committed: false,
            canceled: false,
        }))
    }
}
impl Directory {
    fn descend(&self, name: &OsStr, create: bool) -> LocalResult<Arc<dyn LocalDirectory>> {
        check_name(name)?;
        let path = self.path.join(name);
        before(&self.shared, FsOperation::CreateDir, &path)?;
        let id = lock(&self.shared.tree)
            .held_child(self.id, name, create)
            .map_err(|error| map(error, &path))?;
        Ok(Arc::new(Self {
            shared: self.shared.clone(),
            id,
            path,
        }))
    }
}
struct Cursor {
    names: VecDeque<OsString>,
}
impl LocalCursor for Cursor {
    fn next(&mut self) -> LocalResult<Option<OsString>> {
        Ok(self.names.pop_front())
    }
}
fn check_name(name: &OsStr) -> LocalResult<()> {
    if valid_native_name(name) {
        Ok(())
    } else {
        Err(LocalFsError::InvalidName)
    }
}

pub(super) fn browse(
    shared: Arc<Shared>,
    path: &Path,
    limits: ListingLimits,
    cancel: &CancelFlag,
) -> LocalResult<LocalListing> {
    cancel.check()?;
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        base().join(path)
    };
    before(&shared, FsOperation::List, &path)?;
    let id = lock(&shared.tree)
        .browse_id(&path)
        .map_err(|error| map(error, &path))?;
    let directory = Directory {
        shared,
        id,
        path: path.clone(),
    };
    let mut cursor = directory.entries()?;
    let mut entries = Vec::new();
    let mut names = 0usize;
    while let Some(name) = cursor.next()? {
        cancel.check()?;
        let mut metadata = directory
            .metadata(&name)?
            .ok_or_else(|| LocalFsError::NotFound(path.join(&name)))?;
        names = names.saturating_add(name.to_string_lossy().len());
        if entries.len() >= limits.entries() || names > limits.name_bytes() {
            return Err(LocalFsError::BoundExceeded("Explorer listing"));
        }
        let completion_directory =
            lock(&directory.shared.tree).held_completion_directory(directory.id, &name);
        if metadata.is_symlink {
            metadata.kind = crate::EntryKind::Other;
        }
        entries.push(LocalDirEntry {
            name,
            metadata,
            completion_directory,
        });
    }
    entries.sort_by_cached_key(|entry| {
        (
            entry.metadata.kind != crate::EntryKind::Directory,
            entry.name.to_string_lossy().to_lowercase(),
            entry.name.clone(),
        )
    });
    Ok(LocalListing {
        directory: path,
        entries,
    })
}
pub(super) fn source(shared: Arc<Shared>, path: &Path) -> LocalResult<Box<dyn LocalSource>> {
    let path = resolve(path)?;
    before(&shared, FsOperation::Download, &path)?;
    let (metadata, bytes) = {
        let tree = lock(&shared.tree);
        let metadata = tree
            .metadata_native(&path)
            .map_err(|error| map(error, &path))?;
        if metadata.is_symlink {
            return Err(LocalFsError::LinkRefused(path));
        }
        if metadata.kind != crate::EntryKind::File {
            return Err(LocalFsError::NotRegular(path));
        }
        let bytes = tree
            .bytes_native(&path)
            .map_err(|error| map(error, &path))?;
        (metadata, bytes)
    };
    opened_source(shared, path, metadata, bytes)
}
fn opened_source(
    shared: Arc<Shared>,
    path: PathBuf,
    metadata: FileMetadata,
    bytes: Vec<u8>,
) -> LocalResult<Box<dyn LocalSource>> {
    let mut activity = lock(&shared.activity);
    activity.live_downloads = activity.live_downloads.saturating_add(1);
    activity.peak_downloads = activity.peak_downloads.max(activity.live_downloads);
    drop(activity);
    Ok(Box::new(Source {
        shared,
        path,
        metadata,
        bytes,
        offset: 0,
    }))
}
struct Source {
    shared: Arc<Shared>,
    path: PathBuf,
    metadata: FileMetadata,
    bytes: Vec<u8>,
    offset: usize,
}
impl LocalSource for Source {
    fn metadata(&self) -> LocalResult<FileMetadata> {
        Ok(self.metadata.clone())
    }
    fn read(&mut self, max: usize) -> LocalResult<Vec<u8>> {
        before(&self.shared, FsOperation::Read, &self.path)?;
        if max == 0 || max > 32 * 1024 {
            return Err(LocalFsError::BoundExceeded("source chunk"));
        }
        let end = self
            .offset
            .saturating_add(max.min(lock(&self.shared.limits).chunk))
            .min(self.bytes.len());
        let bytes = self.bytes[self.offset..end].to_vec();
        self.offset = end;
        Ok(bytes)
    }
    fn close(self: Box<Self>) -> LocalResult<()> {
        before(&self.shared, FsOperation::Close, &self.path)?;
        let mut activity = lock(&self.shared.activity);
        activity.closed_downloads = activity.closed_downloads.saturating_add(1);
        Ok(())
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let mut activity = lock(&self.shared.activity);
        activity.live_downloads = activity.live_downloads.saturating_sub(1);
    }
}
struct Stage {
    shared: Arc<Shared>,
    id: u64,
    path: PathBuf,
    bytes: Vec<u8>,
    synced: bool,
    committed: bool,
    canceled: bool,
}
impl Stage {
    fn writable(&self) -> LocalResult<()> {
        if self.committed || self.canceled {
            Err(LocalFsError::Unsupported("a consumed stage"))
        } else {
            Ok(())
        }
    }
}
impl LocalStage for Stage {
    fn write(&mut self, bytes: &[u8]) -> LocalResult<()> {
        self.writable()?;
        before(&self.shared, FsOperation::Write, &self.path)?;
        let limits = lock(&self.shared.limits);
        if bytes.len() > limits.chunk || self.bytes.len().saturating_add(bytes.len()) > limits.file
        {
            return Err(LocalFsError::BoundExceeded("stage bytes"));
        }
        self.bytes.extend_from_slice(bytes);
        self.synced = false;
        Ok(())
    }
    fn sync(&mut self) -> LocalResult<()> {
        self.writable()?;
        before(&self.shared, FsOperation::Sync, &self.path)?;
        self.synced = true;
        Ok(())
    }
    fn publish(&mut self, name: &OsStr, mode: UploadMode) -> LocalResult<PublicationOutcome> {
        self.writable()?;
        check_name(name)?;
        if !self.synced {
            self.sync()?;
        }
        before(&self.shared, FsOperation::Finish, &self.path.join(name))?;
        lock(&self.shared.tree)
            .held_publish(self.id, name, mode, self.bytes.clone())
            .map_err(|error| map(error, &self.path.join(name)))?;
        self.committed = true;
        self.bytes.clear();
        let mut activity = lock(&self.shared.activity);
        activity.completed_uploads = activity.completed_uploads.saturating_add(1);
        drop(activity);
        match before(&self.shared, FsOperation::DirectorySync, &self.path) {
            Ok(()) => Ok(PublicationOutcome::Published),
            Err(error) => Ok(PublicationOutcome::PublishedWithWarning(
                DurabilityWarning::new(std::io::Error::other(error)),
            )),
        }
    }
    fn cancel(mut self: Box<Self>) -> LocalResult<()> {
        self.writable()?;
        before(&self.shared, FsOperation::Cancel, &self.path)?;
        self.canceled = true;
        self.bytes.clear();
        let mut activity = lock(&self.shared.activity);
        activity.canceled_uploads = activity.canceled_uploads.saturating_add(1);
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let mut activity = lock(&self.shared.activity);
        activity.live_uploads = activity.live_uploads.saturating_sub(1);
        if !self.committed && !self.canceled {
            activity.abandoned_uploads = activity.abandoned_uploads.saturating_add(1);
        }
    }
}

pub(super) fn metadata(shared: &Shared, path: &Path) -> LocalResult<FileMetadata> {
    if !capabilities(shared).metadata {
        return Err(LocalFsError::Unsupported("detailed file metadata"));
    }
    let path = resolve(path)?;
    before(shared, FsOperation::Metadata, &path)?;
    lock(&shared.tree)
        .metadata_native(&path)
        .map_err(|error| map(error, &path))
}
pub(super) fn rename(shared: &Shared, path: &Path, name: &OsStr) -> LocalResult<()> {
    if !capabilities(shared).rename {
        return Err(LocalFsError::Unsupported("file rename"));
    }
    check_name(name)?;
    let path = resolve(path)?;
    before(shared, FsOperation::Rename, &path)?;
    lock(&shared.tree)
        .rename_native(&path, name)
        .map_err(|error| map(error, &path))
}
pub(super) fn permissions(shared: &Shared, path: &Path, mode: u32) -> LocalResult<()> {
    if !capabilities(shared).set_permissions {
        return Err(LocalFsError::Unsupported("file permissions"));
    }
    let path = resolve(path)?;
    before(shared, FsOperation::Permissions, &path)?;
    lock(&shared.tree)
        .permissions_native(&path, mode)
        .map_err(|error| map(error, &path))
}

pub(super) fn capabilities(shared: &Shared) -> FsCapabilities {
    *lock(&shared.capabilities)
}

struct ScanFrame {
    directory: Arc<dyn LocalDirectory>,
    cursor: Box<dyn LocalCursor>,
    depth: usize,
}
impl ScanFrame {
    fn new(directory: Arc<dyn LocalDirectory>, depth: usize) -> LocalResult<Self> {
        let cursor = directory.entries()?;
        Ok(Self {
            directory,
            cursor,
            depth,
        })
    }
}
pub(super) fn scan(
    shared: Arc<Shared>,
    path: &Path,
    policy: &ScanPolicy,
    cancel: &CancelFlag,
    progress: &dyn StatisticsObserver,
) -> LocalResult<Statistics> {
    let mut count = StatisticsAccumulator::new(policy.clone());
    if cancel.is_cancelled() {
        let result = count.cancelled();
        progress.update(result.clone());
        return Ok(result);
    }
    let root = match directory(shared, path).and_then(|directory| ScanFrame::new(directory, 0)) {
        Ok(root) => root,
        Err(error) => {
            count.record_failure(path.to_owned(), error);
            let result = count.finish();
            progress.update(result.clone());
            return Ok(result);
        }
    };
    let mut pending = vec![root];
    while let Some(frame) = pending.last_mut() {
        if cancel.is_cancelled() {
            let result = count.cancelled();
            progress.update(result.clone());
            return Ok(result);
        }
        let name = match frame.cursor.next() {
            Ok(Some(name)) => name,
            Ok(None) => {
                pending.pop();
                continue;
            }
            Err(error) => {
                count.record_failure(frame.directory.location(), error);
                pending.pop();
                continue;
            }
        };
        let metadata = match frame.directory.metadata(&name) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                let missing = frame.directory.location().join(&name);
                if count.observe_entry_failure(missing.clone(), LocalFsError::NotFound(missing))
                    == ScanDecision::Stop
                {
                    break;
                }
                continue;
            }
            Err(error) => {
                if count.observe_entry_failure(frame.directory.location().join(name), error)
                    == ScanDecision::Stop
                {
                    break;
                }
                continue;
            }
        };
        match count.observe(&metadata, frame.depth + 1, frame.depth == 0, &name) {
            ScanDecision::Enter => {
                let depth = frame.depth + 1;
                let child_path = frame.directory.location().join(&name);
                match frame
                    .directory
                    .child(&name)
                    .and_then(|directory| ScanFrame::new(directory, depth))
                {
                    Ok(frame) => pending.push(frame),
                    Err(error) => count.record_failure(child_path, error),
                }
            }
            ScanDecision::Skip => {}
            ScanDecision::Stop => break,
        }
    }
    let result = count.finish();
    progress.update(result.clone());
    Ok(result)
}
struct Removal {
    shared: Arc<Shared>,
    limits: RemovalLimits,
    cancel: CancelFlag,
    visited: usize,
    removed: usize,
}
impl Removal {
    fn entry(&mut self, directory: &Directory, name: &OsStr, depth: usize) -> LocalResult<()> {
        self.cancel.check()?;
        self.visited = self.visited.saturating_add(1);
        if depth > self.limits.depth() || self.visited > self.limits.entries() {
            return Err(LocalFsError::BoundExceeded("deletion"));
        }
        let path = directory.path.join(name);
        before(&self.shared, FsOperation::RemoveFile, &path)?;
        let metadata = directory
            .metadata(name)?
            .ok_or_else(|| LocalFsError::NotFound(path.clone()))?;
        let is_directory = metadata.kind == crate::EntryKind::Directory && !metadata.is_symlink;
        if is_directory {
            let id = lock(&self.shared.tree)
                .held_child(directory.id, name, false)
                .map_err(|error| map(error, &path))?;
            let child = Directory {
                shared: self.shared.clone(),
                id,
                path: path.clone(),
            };
            let mut cursor = child.entries()?;
            while let Some(name) = cursor.next()? {
                self.entry(&child, &name, depth + 1)?;
            }
        }
        self.cancel.check()?;
        lock(&self.shared.tree)
            .held_remove(directory.id, name, is_directory)
            .map_err(|error| map(error, &path))?;
        self.removed = self.removed.saturating_add(1);
        Ok(())
    }
}
pub(super) fn remove(
    shared: Arc<Shared>,
    path: &Path,
    limits: RemovalLimits,
    cancel: &CancelFlag,
) -> LocalResult<RemovalOutcome> {
    if !capabilities(&shared).remove {
        return Err(LocalFsError::Unsupported("file removal"));
    }
    cancel.check()?;
    let path = resolve(path)?;
    let name = path
        .file_name()
        .ok_or_else(|| LocalFsError::InvalidPath(path.clone()))?;
    let parent = path
        .parent()
        .ok_or_else(|| LocalFsError::InvalidPath(path.clone()))?;
    let id = lock(&shared.tree)
        .directory_id(parent)
        .map_err(|error| map(error, parent))?;
    let directory = Directory {
        shared: shared.clone(),
        id,
        path: parent.to_owned(),
    };
    let mut removal = Removal {
        shared,
        limits,
        cancel: cancel.clone(),
        visited: 0,
        removed: 0,
    };
    match removal.entry(&directory, name, 0) {
        Ok(()) => Ok(RemovalOutcome {
            removed: removal.removed,
        }),
        Err(error) if removal.removed == 0 => Err(error),
        Err(source) => Err(LocalFsError::PartialRemoval {
            removed: removal.removed,
            source: Box::new(source),
        }),
    }
}
