//! Bounded, session-owned SFTP requests and streaming transfers.
use crate::connection::Client;
use futures::{FutureExt, StreamExt as _, channel::oneshot};
use nocterm_session::{
    DirEntry, EntryKind, FileMetadata, FsCapabilities, FsError, FsFuture, RemoteFs,
    fs::{RemoteDownload, RemoteUpload, UploadMode, path},
};
use russh::client::Handle;
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, FileType, OpenFlags, Packet, StatusCode},
};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinSet,
};
use uuid::Uuid;
type Answer<T> = oneshot::Sender<Result<T, FsError>>;
const REQUEST_BACKLOG: usize = 64;
const CONCURRENT_REQUESTS: usize = 16;
const MAX_CHUNK: usize = 32 * 1024;
const MAX_WRITE_BATCH: usize = 8;
const MAX_OPEN_DOWNLOADS: usize = 128;
const MAX_OPEN_UPLOADS: usize = 128;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRY_NAME: usize = 4096;

pub(crate) enum FsRequest {
    Home(Answer<String>),
    ReadDir(String, Answer<Vec<DirEntry>>),
    Stat(String, Answer<Option<DirEntry>>),
    Metadata(String, Answer<FileMetadata>),
    Rename(String, String, Answer<()>),
    RemoveFile(String, Answer<()>),
    RemoveDir(String, Answer<()>),
    SetPermissions(String, u32, Answer<()>),
    CreateDir(String, Answer<()>),
    Upload(String, UploadMode, Answer<Box<dyn RemoteUpload>>),
    WriteBatch(Uuid, Vec<Vec<u8>>, Answer<()>),
    Finish(Uuid, Answer<String>),
    Cancel(Uuid, Answer<()>),
    Download(String, Answer<Box<dyn RemoteDownload>>),
    Read(Uuid, usize, Answer<Vec<u8>>),
    CloseDownload(Uuid, Answer<()>),
}
enum Cleanup {
    Upload(Uuid),
    Download(Uuid),
}
#[derive(Clone)]
pub(crate) struct SshFs {
    requests: mpsc::Sender<FsRequest>,
    cleanup: mpsc::UnboundedSender<Cleanup>,
}
pub(crate) struct FsRequests {
    requests: mpsc::Receiver<FsRequest>,
    cleanup: mpsc::UnboundedReceiver<Cleanup>,
    sender: SshFs,
}
pub(crate) fn channel() -> (SshFs, FsRequests) {
    let (requests, receiver) = mpsc::channel(REQUEST_BACKLOG);
    // Cleanup notices carry identities only; file data never uses this channel.
    let (cleanup, cleanup_receiver) = mpsc::unbounded_channel();
    let sender = SshFs { requests, cleanup };
    (
        sender.clone(),
        FsRequests {
            requests: receiver,
            cleanup: cleanup_receiver,
            sender,
        },
    )
}
impl SshFs {
    fn request<T: Send + 'static>(
        &self,
        request: impl FnOnce(Answer<T>) -> FsRequest + Send + 'static,
    ) -> FsFuture<T> {
        let sender = self.requests.clone();
        async move {
            let (answer, result) = oneshot::channel();
            sender
                .send(request(answer))
                .await
                .map_err(|_| FsError::NotConnected)?;
            result.await.unwrap_or(Err(FsError::NotConnected))
        }
        .boxed()
    }
}
impl RemoteFs for SshFs {
    fn capabilities(&self) -> FsCapabilities {
        let available = !self.requests.is_closed();
        FsCapabilities {
            metadata: available,
            rename: available,
            remove: available,
            set_permissions: available,
        }
    }
    fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::Metadata(path, reply))
    }
    fn rename(&self, old: &str, new: &str) -> FsFuture<()> {
        let old = old.to_owned();
        let new = new.to_owned();
        self.request(move |reply| FsRequest::Rename(old, new, reply))
    }
    fn remove_file(&self, path: &str) -> FsFuture<()> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::RemoveFile(path, reply))
    }
    fn remove_dir(&self, path: &str) -> FsFuture<()> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::RemoveDir(path, reply))
    }
    fn set_permissions(&self, path: &str, permissions: u32) -> FsFuture<()> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::SetPermissions(path, permissions, reply))
    }
    fn home(&self) -> FsFuture<String> {
        self.request(FsRequest::Home)
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::ReadDir(path, reply))
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::Stat(path, reply))
    }
    fn create_dir(&self, path: &str) -> FsFuture<()> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::CreateDir(path, reply))
    }
    fn upload(&self, path: &str, mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::Upload(path, mode, reply))
    }
    fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        let path = path.to_owned();
        self.request(move |reply| FsRequest::Download(path, reply))
    }
}
struct Download {
    source: String,
    handle: Option<String>,
    offset: u64,
    eof: bool,
}
type Downloads = Arc<Mutex<HashMap<Uuid, Arc<Mutex<Download>>>>>;

struct Upload {
    temporary: String,
    destination: String,
    handle: Option<String>,
    offset: u64,
    mode: UploadMode,
    failed: bool,
}
type Uploads = Arc<Mutex<HashMap<Uuid, Arc<Mutex<Upload>>>>>;
struct Sftp {
    raw: RawSftpSession,
    atomic_replace: bool,
}

mod actor;
mod download;
mod io;
mod listing;
mod ops;
#[cfg(test)]
mod tests;
mod upload;

use self::{download::*, io::*, listing::*, ops::*, upload::*};
pub(crate) use actor::serve;
