//! The remote file system, as far as nocterm browses it.

use futures::future::BoxFuture;

/// The result of a file system request, resolved off the calling thread.
pub type FsFuture<T> = BoxFuture<'static, Result<T, FsError>>;

/// A file system on the other end of a session.
pub trait RemoteFs: Send + Sync + 'static {
    /// The directory a new shell starts in, as an absolute path.
    fn home(&self) -> FsFuture<String>;

    /// The entries of a directory, without `.` and `..`, in no particular order.
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>>;

    /// Metadata for the path itself, without following its final symlink.
    fn stat(&self, _path: &str) -> FsFuture<Option<DirEntry>> {
        Box::pin(async { Err(FsError::Unsupported("file metadata".into())) })
    }

    /// Create a directory or accept an existing real directory.
    fn create_dir(&self, _path: &str) -> FsFuture<()> {
        Box::pin(async { Err(FsError::Unsupported("directory creation".into())) })
    }

    /// Start a staged upload. The final path is untouched until `finish`.
    fn upload(&self, _path: &str, _mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        Box::pin(async { Err(FsError::Unsupported("file upload".into())) })
    }

    /// Open a regular file for bounded streaming reads. Final symlinks and
    /// special files are rejected; closing or dropping releases its handle.
    fn download(&self, _path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        Box::pin(async { Err(FsError::Unsupported("file download".into())) })
    }
}

/// Publication policy, independent of SFTP implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadMode {
    /// Fail if the destination exists; never silently replace a file.
    Create,
    /// Explicitly authorized atomic replacement, when the server supports it.
    Replace,
}

/// A streaming upload. Callers await each chunk for back-pressure.
/// Dropping it abandons the temporary file; implementations own cleanup.
pub trait RemoteUpload: Send + 'static {
    fn write(&mut self, bytes: Vec<u8>) -> FsFuture<()>;
    /// At most eight 32-KiB chunks. Adapters may pipeline acknowledgements;
    /// success means every chunk was acknowledged, never merely queued.
    fn write_batch(&mut self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        if chunks.len() > 8 || chunks.iter().any(|chunk| chunk.len() > 32 * 1024) {
            return Box::pin(async {
                Err(FsError::Other("upload batch exceeds 8 × 32 KiB".into()))
            });
        }
        let writes: Vec<_> = chunks.into_iter().map(|chunk| self.write(chunk)).collect();
        Box::pin(async move {
            for write in writes {
                write.await?;
            }
            Ok(())
        })
    }
    fn finish(self: Box<Self>) -> FsFuture<String>;
    fn cancel(self: Box<Self>) -> FsFuture<()>;
}

/// An empty read signals EOF. Implementations never return more than the
/// requested bound; callers close after EOF and drop on cancellation.
pub trait RemoteDownload: Send + 'static {
    fn read(&mut self, max_bytes: usize) -> FsFuture<Vec<u8>>;
    fn close(self: Box<Self>) -> FsFuture<()>;
}

/// One entry of a remote directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    /// What the entry is, after following a symbolic link.
    pub kind: EntryKind,
    pub is_symlink: bool,
    /// Size in bytes, when the host reports one.
    pub size: Option<u64>,
}

/// What a directory entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    /// A device, a socket, a link to nowhere: present, but not browsable.
    Other,
}

/// A file system request failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FsError {
    #[error("the session is not connected")]
    NotConnected,
    #[error("{path}: permission denied")]
    PermissionDenied { path: String },
    #[error("{path}: no such file or directory")]
    NotFound { path: String },
    #[error("{path}: already exists")]
    AlreadyExists { path: String },
    #[error("the host does not offer file access: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

/// Operations on remote paths, which are always POSIX paths whatever the
/// local platform is.
pub mod path {
    /// `parent` with `name` appended.
    pub fn join(parent: &str, name: &str) -> String {
        if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        }
    }

    /// The directory containing `path`; the root is its own parent.
    pub fn parent(path: &str) -> &str {
        let trimmed = trim(path);
        match trimmed.rfind('/') {
            Some(0) | None => "/",
            Some(ix) => &trimmed[..ix],
        }
    }

    /// The last component of `path`; the root's name is `/`.
    pub fn file_name(path: &str) -> &str {
        let trimmed = trim(path);
        match trimmed.rfind('/') {
            _ if trimmed == "/" => "/",
            Some(ix) => &trimmed[ix + 1..],
            None => trimmed,
        }
    }

    /// Strips trailing slashes, keeping the root intact.
    fn trim(path: &str) -> &str {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() { "/" } else { trimmed }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn joins_without_doubling_slashes() {
            assert_eq!(join("/", "etc"), "/etc");
            assert_eq!(join("/etc", "ssh"), "/etc/ssh");
            assert_eq!(join("/etc/", "ssh"), "/etc/ssh");
        }

        #[test]
        fn parent_stops_at_the_root() {
            assert_eq!(parent("/home/me/src"), "/home/me");
            assert_eq!(parent("/home/me/"), "/home");
            assert_eq!(parent("/home"), "/");
            assert_eq!(parent("/"), "/");
        }

        #[test]
        fn file_name_is_the_last_component() {
            assert_eq!(file_name("/home/me"), "me");
            assert_eq!(file_name("/home/me/"), "me");
            assert_eq!(file_name("/"), "/");
            assert_eq!(file_name("relative"), "relative");
        }
    }
}
