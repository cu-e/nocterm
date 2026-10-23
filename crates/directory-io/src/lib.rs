//! Read-only directory adapters over existing filesystem capabilities.
//! Owns: captured source selection and bounded directory/home reads.
//! Does not own: native traversal, mutations, request scheduling or GUI state.
//! Ports used: LocalFs, RemoteFs. Ports provided: DirectoryRead, DirectoryReaders.
//! Settings: none. Globals: none.
//! Invariants: no remote-to-local fallback; names and linked parent paths stay literal.
use nocterm_explorer::directory::{
    Directory, DirectoryError, DirectoryFuture, DirectoryListing, DirectoryRead,
};
use nocterm_session::{
    RemoteFs,
    local_fs::{CancelFlag, ListingLimits, LocalFs},
};
use std::sync::Arc;

pub struct LocalDirectoryReader {
    source: Arc<dyn LocalFs>,
}
impl LocalDirectoryReader {
    pub fn new(source: Arc<dyn LocalFs>) -> Self {
        Self { source }
    }
}
impl DirectoryRead for LocalDirectoryReader {
    fn home(&self) -> DirectoryFuture<Directory> {
        let source = self.source.clone();
        Box::pin(async move {
            source
                .home()?
                .map(Directory::Local)
                .ok_or(DirectoryError::HomeUnavailable)
        })
    }
    fn list(&self, directory: Directory, cancel: CancelFlag) -> DirectoryFuture<DirectoryListing> {
        let source = self.source.clone();
        Box::pin(async move {
            let Directory::Local(path) = directory else {
                return Err(DirectoryError::SourceMismatch);
            };
            Ok(DirectoryListing::local(source.browse(
                &path,
                ListingLimits::default(),
                &cancel,
            )?))
        })
    }
}
pub struct RemoteDirectoryReader {
    source: Arc<dyn RemoteFs>,
}
impl RemoteDirectoryReader {
    pub fn new(source: Arc<dyn RemoteFs>) -> Self {
        Self { source }
    }
}
impl DirectoryRead for RemoteDirectoryReader {
    fn home(&self) -> DirectoryFuture<Directory> {
        let source = self.source.clone();
        Box::pin(async move { Ok(Directory::Remote(source.home().await?)) })
    }
    fn list(&self, directory: Directory, cancel: CancelFlag) -> DirectoryFuture<DirectoryListing> {
        let source = self.source.clone();
        Box::pin(async move {
            cancel.check()?;
            let Directory::Remote(path) = directory else {
                return Err(DirectoryError::SourceMismatch);
            };
            let entries = source.read_dir(&path).await?;
            cancel.check()?;
            DirectoryListing::remote(path, entries)
        })
    }
}
/// Retained application composition for views that follow changing session sources.
pub struct DirectoryReaders {
    local: Arc<dyn DirectoryRead>,
}
impl DirectoryReaders {
    pub fn new(local: Arc<dyn LocalFs>) -> Self {
        Self {
            local: Arc::new(LocalDirectoryReader::new(local)),
        }
    }
}
impl nocterm_explorer::directory::DirectoryReaders for DirectoryReaders {
    fn local(&self) -> Arc<dyn DirectoryRead> {
        self.local.clone()
    }
    fn remote(&self, source: Arc<dyn RemoteFs>) -> Arc<dyn DirectoryRead> {
        Arc::new(RemoteDirectoryReader::new(source))
    }
}
#[cfg(test)]
mod tests;
