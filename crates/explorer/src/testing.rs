//! Test-only directory capabilities backed by session's shared in-memory ports.
//! No native traversal or private filesystem tree is implemented here.
use crate::directory::{
    Directory, DirectoryError, DirectoryFuture, DirectoryListing, DirectoryRead, DirectoryReaders,
};
use nocterm_session::{
    RemoteFs,
    local_fs::{CancelFlag, ListingLimits, LocalFs},
};
use std::sync::Arc;

pub struct MemoryDirectoryReaders {
    local: Arc<dyn LocalFs>,
}
impl MemoryDirectoryReaders {
    pub fn new(local: Arc<dyn LocalFs>) -> Self {
        Self { local }
    }
}
impl DirectoryReaders for MemoryDirectoryReaders {
    fn local(&self) -> Arc<dyn DirectoryRead> {
        Arc::new(MemoryDirectoryReader::local(self.local.clone()))
    }
    fn remote(&self, source: Arc<dyn RemoteFs>) -> Arc<dyn DirectoryRead> {
        Arc::new(MemoryDirectoryReader::remote(source))
    }
}
/// Adapts existing session test capabilities; arbitrary native implementations must
/// never be supplied to this test-only owner.
pub struct MemoryDirectoryReader {
    source: MemorySource,
}
enum MemorySource {
    Local(Arc<dyn LocalFs>),
    Remote(Arc<dyn RemoteFs>),
}
impl MemoryDirectoryReader {
    pub fn local(source: Arc<dyn LocalFs>) -> Self {
        Self {
            source: MemorySource::Local(source),
        }
    }
    pub fn remote(source: Arc<dyn RemoteFs>) -> Self {
        Self {
            source: MemorySource::Remote(source),
        }
    }
}
impl DirectoryRead for MemoryDirectoryReader {
    fn home(&self) -> DirectoryFuture<Directory> {
        match &self.source {
            MemorySource::Local(source) => {
                let source = source.clone();
                Box::pin(async move {
                    source
                        .home()?
                        .map(Directory::Local)
                        .ok_or(DirectoryError::HomeUnavailable)
                })
            }
            MemorySource::Remote(source) => {
                let future = source.home();
                Box::pin(async move { Ok(Directory::Remote(future.await?)) })
            }
        }
    }
    fn list(&self, directory: Directory, cancel: CancelFlag) -> DirectoryFuture<DirectoryListing> {
        match (&self.source, directory) {
            (MemorySource::Local(source), Directory::Local(path)) => {
                let source = source.clone();
                Box::pin(async move {
                    Ok(DirectoryListing::local(source.browse(
                        &path,
                        ListingLimits::default(),
                        &cancel,
                    )?))
                })
            }
            (MemorySource::Remote(source), Directory::Remote(path)) => {
                let future = source.read_dir(&path);
                Box::pin(async move {
                    cancel.check()?;
                    DirectoryListing::remote(path, future.await?)
                })
            }
            _ => Box::pin(async { Err(DirectoryError::SourceMismatch) }),
        }
    }
}
