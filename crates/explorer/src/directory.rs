//! Literal navigation rules and a read-only directory capability.
mod error;
mod path;
pub use error::DirectoryError;
use futures::future::BoxFuture;
use nocterm_session::{
    DirEntry, EntryKind, RemoteFs,
    local_fs::{CancelFlag, LocalListing},
};
pub use path::{Candidate, Cycle, Directory, PathError, Query, query, resolve};
use std::sync::Arc;

pub type DirectoryFuture<T> = BoxFuture<'static, Result<T, DirectoryError>>;

/// A captured filesystem source. Futures may block: poll on a background executor.
/// `list` must actually read the directory, reporting permission and type errors.
pub trait DirectoryRead: Send + Sync + 'static {
    fn home(&self) -> DirectoryFuture<Directory>;
    fn list(&self, directory: Directory, cancel: CancelFlag) -> DirectoryFuture<DirectoryListing>;
}

/// Composition supplies source adapters; views never select a native implementation.
pub trait DirectoryReaders: Send + Sync + 'static {
    fn local(&self) -> Arc<dyn DirectoryRead>;
    fn remote(&self, source: Arc<dyn RemoteFs>) -> Arc<dyn DirectoryRead>;
}

#[derive(Clone, Debug)]
pub struct DirectoryListing {
    pub directory: Directory,
    pub candidates: Vec<Candidate>,
}
impl DirectoryListing {
    pub fn local(listing: LocalListing) -> Self {
        let candidates = listing
            .entries
            .into_iter()
            .filter(|entry| entry.completion_directory)
            .filter_map(|entry| entry.name.to_str().map(str::to_owned))
            .filter(|name| valid_name(name, true))
            .map(|name| Candidate {
                name,
                directory: true,
            })
            .collect();
        Self {
            directory: Directory::Local(listing.directory),
            candidates,
        }
    }
    pub fn remote(directory: String, entries: Vec<DirEntry>) -> Result<Self, DirectoryError> {
        if entries.len() > 100_000
            || entries.iter().map(|entry| entry.name.len()).sum::<usize>() > 8 * 1024 * 1024
        {
            return Err(DirectoryError::ListingLimit);
        }
        let candidates = entries
            .into_iter()
            .filter(|entry| entry.kind == EntryKind::Directory && valid_name(&entry.name, false))
            .map(|entry| Candidate {
                name: entry.name,
                directory: true,
            })
            .collect();
        Ok(Self {
            directory: Directory::Remote(directory),
            candidates: sorted(candidates),
        })
    }
}
fn sorted(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.name.clone()));
    candidates
}
pub fn valid_name(name: &str, local: bool) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !(local && cfg!(windows) && name.contains('\\'))
        && !name.chars().any(char::is_control)
}
impl Directory {
    /// A display value cannot replace a native non-UTF8 path's identity.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Local(path) => path.to_str(),
            Self::Remote(path) => Some(path),
        }
    }
    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Local(path) => path.parent().map(|path| Self::Local(path.to_owned())),
            Self::Remote(path) if path != "/" => Some(Self::Remote(
                nocterm_session::fs::path::parent(path).to_owned(),
            )),
            Self::Remote(_) => None,
        }
    }
}
