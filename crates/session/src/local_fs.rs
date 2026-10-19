//! Native local filesystem contracts. Calls are synchronous and must be dispatched
//! off UI/executor threads by the caller; these contracts expose no OS handles.
mod error;
mod policy;
mod statistics;

use crate::{FileMetadata, FsCapabilities, fs::UploadMode};
pub use error::{DurabilityWarning, LocalFsError, LocalResult, PublicationOutcome};
pub use policy::{
    CancelFlag, DirectoryIdentity, ListingLimits, RemovalLimits, ScanPolicy, valid_native_name,
};
pub use statistics::{
    ScanDecision, ScanIssue, ScanSkip, Statistics, StatisticsAccumulator, StatisticsObserver,
    local_skip, remote_skip,
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::Arc,
};

pub trait LocalFs: Send + Sync + 'static {
    fn home(&self) -> LocalResult<Option<PathBuf>>;
    fn resolve_source(&self, path: &Path) -> LocalResult<PathBuf>;
    /// Read-only navigation may follow directory links. Entries keep lstat
    /// metadata; completion eligibility may probe the target without authorizing
    /// mutation. Enforce both listing limits and preserve exact native names.
    fn browse(
        &self,
        path: &Path,
        limits: ListingLimits,
        cancel: &CancelFlag,
    ) -> LocalResult<LocalListing>;
    /// Acquire a strict capability: reject links in every component. Operations
    /// retain held native identity where the platform supports it. A path-based
    /// adapter must document and recheck its active-ancestor race limitation.
    fn open_directory(&self, path: &Path) -> LocalResult<Arc<dyn LocalDirectory>>;
    /// Open a regular file without following links or waiting on a replaced FIFO.
    /// Metadata and bytes refer to this opened source, rather than later paths.
    fn open_source(&self, path: &Path) -> LocalResult<Box<dyn LocalSource>>;
    fn scan(
        &self,
        path: &Path,
        policy: &ScanPolicy,
        cancel: &CancelFlag,
        progress: &dyn StatisticsObserver,
    ) -> LocalResult<Statistics>;
}
pub trait LocalMutations: Send + Sync + 'static {
    fn capabilities(&self) -> FsCapabilities;
    fn metadata(&self, path: &Path) -> LocalResult<FileMetadata>;
    fn rename(&self, path: &Path, name: &OsStr) -> LocalResult<()>;
    fn set_permissions(&self, path: &Path, mode: u32) -> LocalResult<()>;
    fn remove(
        &self,
        path: &Path,
        limits: RemovalLimits,
        cancel: &CancelFlag,
    ) -> LocalResult<RemovalOutcome>;
}
pub trait LocalDirectory: Send + Sync + 'static {
    /// Display/navigation location only; never use it to replace held operations.
    fn location(&self) -> PathBuf;
    fn identity(&self) -> DirectoryIdentity;
    fn entries(&self) -> LocalResult<Box<dyn LocalCursor>>;
    fn metadata(&self, name: &OsStr) -> LocalResult<Option<FileMetadata>>;
    fn child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>>;
    fn create_child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>>;
    /// Open relative to this held directory, preserving its identity after rename.
    fn open_source(&self, name: &OsStr) -> LocalResult<Box<dyn LocalSource>>;
    fn stage(&self) -> LocalResult<Box<dyn LocalStage>>;
}
pub trait LocalCursor: Send + 'static {
    fn next(&mut self) -> LocalResult<Option<OsString>>;
}
pub trait LocalSource: Send + 'static {
    fn metadata(&self) -> LocalResult<FileMetadata>;
    /// `max_bytes` must be 1..=32 KiB. Empty bytes indicate EOF.
    fn read(&mut self, max_bytes: usize) -> LocalResult<Vec<u8>>;
    fn close(self: Box<Self>) -> LocalResult<()>;
}
pub trait LocalStage: Send + 'static {
    /// Each write is bounded to 32 KiB; bytes remain private until publication.
    fn write(&mut self, bytes: &[u8]) -> LocalResult<()>;
    fn sync(&mut self) -> LocalResult<()>;
    /// An error guarantees no final commit from this call. Committed durability
    /// warnings count as publication success, never as a retryable failed rename.
    /// A no-clobber collision keeps the stage available for another name; any
    /// successful publication consumes it and forbids publishing twice.
    fn publish(&mut self, name: &OsStr, mode: UploadMode) -> LocalResult<PublicationOutcome>;
    /// Success confirms removal or that the private candidate is already absent.
    /// Cleanup failure is returned explicitly; Drop alone is best effort.
    fn cancel(self: Box<Self>) -> LocalResult<()>;
}
#[derive(Clone, Debug)]
pub struct LocalDirEntry {
    pub name: OsString,
    pub metadata: FileMetadata,
    /// A read-only symlink-target probe used by path completion.
    pub completion_directory: bool,
}
#[derive(Clone, Debug)]
pub struct LocalListing {
    pub directory: PathBuf,
    pub entries: Vec<LocalDirEntry>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemovalOutcome {
    pub removed: usize,
}

#[cfg(test)]
mod tests;
