//! Bounded, session-owned SFTP requests and streaming transfers.
use crate::connection::Client;
use futures::{FutureExt, channel::oneshot};
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
struct Writer {
    id: Uuid,
    fs: SshFs,
}
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.fs.cleanup.send(Cleanup::Upload(self.id));
    }
}
impl RemoteUpload for Writer {
    fn write(&mut self, bytes: Vec<u8>) -> FsFuture<()> {
        self.write_batch(vec![bytes])
    }
    fn write_batch(&mut self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        if let Err(error) = validate_batch(&chunks) {
            return async move { Err(error) }.boxed();
        }
        let id = self.id;
        self.fs
            .request(move |reply| FsRequest::WriteBatch(id, chunks, reply))
    }
    fn finish(self: Box<Self>) -> FsFuture<String> {
        async move {
            self.fs
                .request({
                    let id = self.id;
                    move |reply| FsRequest::Finish(id, reply)
                })
                .await
        }
        .boxed()
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        async move {
            self.fs
                .request({
                    let id = self.id;
                    move |reply| FsRequest::Cancel(id, reply)
                })
                .await
        }
        .boxed()
    }
}
struct Reader {
    id: Uuid,
    fs: SshFs,
}
impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.fs.cleanup.send(Cleanup::Download(self.id));
    }
}
impl RemoteDownload for Reader {
    fn read(&mut self, max_bytes: usize) -> FsFuture<Vec<u8>> {
        if max_bytes == 0 {
            return async { Err(FsError::Other("download read size must be positive".into())) }
                .boxed();
        }
        let id = self.id;
        let max_bytes = max_bytes.min(MAX_CHUNK);
        self.fs
            .request(move |reply| FsRequest::Read(id, max_bytes, reply))
    }
    fn close(self: Box<Self>) -> FsFuture<()> {
        async move {
            self.fs
                .request({
                    let id = self.id;
                    move |reply| FsRequest::CloseDownload(id, reply)
                })
                .await
        }
        .boxed()
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

/// Worker tasks are owned by the session. Shutdown cancels requests, then
/// removes only this session's partial files before SSH disconnect.
pub(crate) async fn serve(
    handle: Arc<Handle<Client>>,
    mut requests: FsRequests,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut opened: Option<Result<Arc<Sftp>, FsError>> = None;
    let uploads: Uploads = Arc::default();
    let downloads: Downloads = Arc::default();
    let mut workers = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(_) = workers.join_next(), if !workers.is_empty() => {},
            Some(cleanup) = requests.cleanup.recv(), if workers.len() < CONCURRENT_REQUESTS => {
                if let Some(Ok(sftp)) = &opened {
                    let uploads = uploads.clone(); let downloads = downloads.clone(); let sftp = sftp.clone();
                    workers.spawn(async move { match cleanup {
                        Cleanup::Upload(id) => { let _ = cancel(&sftp, &uploads, id).await; }
                        Cleanup::Download(id) => { let _ = close_download(&sftp, &downloads, id).await; }
                    } });
                }
            },
            request = requests.requests.recv(), if workers.len() < CONCURRENT_REQUESTS => {
                let Some(request) = request else { break; };
                let sftp = match &opened {
                    Some(result) => result.clone(),
                    None => opened.insert(open(&handle).await.map(Arc::new)).clone(),
                };
                let uploads = uploads.clone();
                let downloads = downloads.clone();
                let sender = requests.sender.clone();
                workers.spawn(async move { dispatch(request, sftp, uploads, downloads, sender).await });
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    if let Some(Ok(sftp)) = opened {
        let ids: Vec<_> = uploads.lock().await.keys().copied().collect();
        for id in ids {
            let _ = cancel(&sftp, &uploads, id).await;
        }
        let ids: Vec<_> = downloads.lock().await.keys().copied().collect();
        for id in ids {
            let _ = close_download(&sftp, &downloads, id).await;
        }
        let _ = sftp.raw.close_session();
    }
}
async fn dispatch(
    request: FsRequest,
    result: Result<Arc<Sftp>, FsError>,
    uploads: Uploads,
    downloads: Downloads,
    fs: SshFs,
) {
    match request {
        FsRequest::Home(reply) => {
            let _ = reply.send(match result {
                Ok(s) => home(&s).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::ReadDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => read_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Stat(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => stat(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Metadata(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => metadata(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Rename(old, new, reply) => {
            let _ = reply.send(match result {
                Ok(s) => rename(&s, &old, &new).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::RemoveFile(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => remove_file(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::RemoveDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => remove_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::SetPermissions(path, permissions, reply) => {
            let _ = reply.send(match result {
                Ok(s) => set_permissions(&s, &path, permissions).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::CreateDir(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => create_dir(&s, &path).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Upload(path, mode, reply) => {
            let _ = reply.send(match result {
                Ok(s) => start_upload(&s, &uploads, path, mode, fs).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::WriteBatch(id, chunks, reply) => {
            let _ = reply.send(match result {
                Ok(s) => write_batch(&s, &uploads, id, chunks).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Finish(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => finish(&s, &uploads, id).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Cancel(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => cancel(&s, &uploads, id).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Download(path, reply) => {
            let _ = reply.send(match result {
                Ok(s) => start_download(&s, &downloads, path, fs).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::Read(id, max_bytes, reply) => {
            let _ = reply.send(match result {
                Ok(s) => read(&s, &downloads, id, max_bytes).await,
                Err(e) => Err(e),
            });
        }
        FsRequest::CloseDownload(id, reply) => {
            let _ = reply.send(match result {
                Ok(s) => close_download(&s, &downloads, id).await,
                Err(e) => Err(e),
            });
        }
    }
}
async fn open(handle: &Handle<Client>) -> Result<Sftp, FsError> {
    let unsupported = |e: &dyn std::fmt::Display| FsError::Unsupported(e.to_string());
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| unsupported(&e))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| unsupported(&e))?;
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await.map_err(|e| unsupported(&e))?;
    Ok(Sftp {
        raw,
        atomic_replace: version
            .extensions
            .get("posix-rename@openssh.com")
            .is_some_and(|v| v == "1"),
    })
}
async fn home(sftp: &Sftp) -> Result<String, FsError> {
    sftp.raw
        .realpath(".")
        .await
        .map_err(|e| fs_error(e, "."))?
        .files
        .into_iter()
        .next()
        .map(|f| f.filename)
        .ok_or_else(|| FsError::Other("missing remote home directory".into()))
}
async fn stat(sftp: &Sftp, path: &str) -> Result<Option<DirEntry>, FsError> {
    match sftp.raw.lstat(path).await {
        Ok(attrs) => Ok(Some(entry(path::file_name(path).into(), &attrs.attrs))),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => Ok(None),
        Err(error) => Err(fs_error(error, path)),
    }
}
fn entry(name: String, attrs: &FileAttributes) -> DirEntry {
    DirEntry {
        name,
        kind: entry_kind(attrs.file_type()),
        is_symlink: attrs.file_type() == FileType::Symlink,
        size: attrs.size,
    }
}
fn checked_path(path: &str, mutation: bool) -> Result<&str, FsError> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(FsError::Other(
            "remote paths must be absolute without NUL".into(),
        ));
    }
    let trimmed = path.trim_end_matches('/');
    if mutation && (trimmed.is_empty() || path.split('/').any(|part| part == "." || part == "..")) {
        return Err(FsError::Other(
            "cannot modify the remote root or dot path components".into(),
        ));
    }
    Ok(if trimmed.is_empty() { "/" } else { trimmed })
}
async fn metadata(sftp: &Sftp, path: &str) -> Result<FileMetadata, FsError> {
    let path = checked_path(path, false)?;
    let attrs = sftp
        .raw
        .lstat(path)
        .await
        .map_err(|e| fs_error(e, path))?
        .attrs;
    Ok(FileMetadata {
        kind: entry_kind(attrs.file_type()),
        is_symlink: attrs.file_type() == FileType::Symlink,
        size: attrs.size,
        permissions: attrs.permissions.map(|permissions| permissions & 0o7777),
        uid: attrs.uid,
        gid: attrs.gid,
        modified: attrs.mtime.map(u64::from),
    })
}
async fn rename(sftp: &Sftp, old: &str, new: &str) -> Result<(), FsError> {
    let old = checked_path(old, true)?;
    let new = checked_path(new, true)?;
    // SFTP v3 RENAME refuses existing destinations; never select the replacing
    // posix-rename extension used by explicitly authorized transfer Replace.
    if stat(sftp, new).await?.is_some() {
        return Err(FsError::AlreadyExists { path: new.into() });
    }
    sftp.raw
        .rename(old, new)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, old))
}
async fn remove_file(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    let attrs = metadata(sftp, path).await?;
    if attrs.kind == EntryKind::Directory && !attrs.is_symlink {
        return Err(FsError::Other(format!(
            "{path}: is a directory; use empty-directory removal"
        )));
    }
    sftp.raw
        .remove(path)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
async fn remove_dir(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    let attrs = metadata(sftp, path).await?;
    if attrs.kind != EntryKind::Directory || attrs.is_symlink {
        return Err(FsError::Other(format!("{path}: is not a real directory")));
    }
    sftp.raw
        .rmdir(path)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
async fn set_permissions(sftp: &Sftp, path: &str, permissions: u32) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    if permissions & !0o7777 != 0 {
        return Err(FsError::Other(
            "permissions must contain only POSIX bits 0o0000..0o7777".into(),
        ));
    }
    let attrs = sftp
        .raw
        .lstat(path)
        .await
        .map_err(|e| fs_error(e, path))?
        .attrs;
    if attrs.permissions.is_none_or(|mode| mode & 0o170000 == 0)
        || attrs.file_type() == FileType::Symlink
    {
        return Err(FsError::Unsupported(
            "changing permissions on links or objects with unknown type".into(),
        ));
    }
    // SFTP v3 has no no-follow SETSTAT. This rejects a link observed by LSTAT,
    // but an external server-side replacement between these requests cannot be
    // excluded by this protocol. Only permissions are sent; size/owner/time stay.
    let mut updated = FileAttributes::empty();
    updated.permissions = Some(permissions);
    sftp.raw
        .setstat(path, updated)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}

async fn read_dir(sftp: &Sftp, dir: &str) -> Result<Vec<DirEntry>, FsError> {
    let handle = sftp
        .raw
        .opendir(dir)
        .await
        .map_err(|e| fs_error(e, dir))?
        .handle;
    let result = async {
        let mut entries = Vec::new();
        loop {
            match sftp.raw.readdir(&handle).await {
                Ok(names) => {
                    for name in names.files {
                        if name.filename == "." || name.filename == ".." {
                            continue;
                        }
                        let mut item = entry(name.filename, &name.attrs);
                        if item.is_symlink
                            && let Ok(target) = sftp.raw.stat(path::join(dir, &item.name)).await
                        {
                            item.kind = entry_kind(target.attrs.file_type());
                            item.size = target.attrs.size;
                        }
                        entries.push(item);
                    }
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => break,
                Err(e) => return Err(fs_error(e, dir)),
            }
        }
        Ok(entries)
    }
    .await;
    let closed = sftp.raw.close(handle).await.map_err(|e| fs_error(e, dir));
    closed?;
    result
}
async fn create_dir(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    if let Some(entry) = stat(sftp, path).await? {
        return if entry.kind == EntryKind::Directory && !entry.is_symlink {
            Ok(())
        } else {
            Err(FsError::AlreadyExists { path: path.into() })
        };
    }
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o755);
    sftp.raw
        .mkdir(path, attrs)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
async fn start_upload(
    sftp: &Sftp,
    uploads: &Uploads,
    destination: String,
    mode: UploadMode,
    fs: SshFs,
) -> Result<Box<dyn RemoteUpload>, FsError> {
    if mode == UploadMode::Replace && !sftp.atomic_replace {
        return Err(FsError::Unsupported(
            "this server cannot atomically replace files; choose Rename or Skip".into(),
        ));
    }
    if let Some(existing) = stat(sftp, &destination).await?
        && (mode == UploadMode::Create || existing.kind != EntryKind::File || existing.is_symlink)
    {
        return Err(FsError::AlreadyExists { path: destination });
    }
    let id = Uuid::new_v4();
    let temporary = path::join(path::parent(&destination), &format!(".nocterm-{id}.part"));
    let upload = Arc::new(Mutex::new(Upload {
        temporary: temporary.clone(),
        destination,
        handle: None,
        offset: 0,
        mode,
        failed: false,
    }));
    uploads.lock().await.insert(id, upload.clone());
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o600);
    let opened = sftp
        .raw
        .open(
            &temporary,
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            attrs,
        )
        .await;
    match opened {
        Ok(handle) => {
            upload.lock().await.handle = Some(handle.handle);
            Ok(Box::new(Writer { id, fs }))
        }
        Err(e) => {
            uploads.lock().await.remove(&id);
            Err(fs_error(e, &temporary))
        }
    }
}
async fn get_upload(uploads: &Uploads, id: Uuid) -> Result<Arc<Mutex<Upload>>, FsError> {
    uploads
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| FsError::Other("upload already finished or cancelled".into()))
}
fn validate_batch(chunks: &[Vec<u8>]) -> Result<(), FsError> {
    if chunks.len() > MAX_WRITE_BATCH || chunks.iter().any(|chunk| chunk.len() > MAX_CHUNK) {
        return Err(FsError::Other("upload batch exceeds 8 × 32 KiB".into()));
    }
    Ok(())
}
async fn write_batch(
    sftp: &Sftp,
    uploads: &Uploads,
    id: Uuid,
    chunks: Vec<Vec<u8>>,
) -> Result<(), FsError> {
    validate_batch(&chunks)?;
    let upload = get_upload(uploads, id).await?;
    // One writer owns the complete contiguous offset range until every
    // acknowledgement arrives. Other writes and finish cannot overtake it.
    let mut upload = upload.lock().await;
    if upload.failed {
        return Err(FsError::Other(
            "upload failed; cancel and start again".into(),
        ));
    }
    let handle = upload.handle.clone().ok_or(FsError::NotConnected)?;
    let mut end = upload.offset;
    let mut writes = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let offset = end;
        let Some(next) = end.checked_add(chunk.len() as u64) else {
            upload.failed = true;
            return Err(FsError::Other("upload offset overflow".into()));
        };
        end = next;
        writes.push(sftp.raw.write(handle.clone(), offset, chunk));
    }
    // Unlike try_join_all, this drains all replies even after one fails.
    // Never publish a partial batch or release the writer with pending ACKs.
    let replies = futures::future::join_all(writes).await;
    if let Some(error) = replies.into_iter().find_map(Result::err) {
        upload.failed = true;
        return Err(fs_error(error, &upload.destination));
    }
    upload.offset = end;
    Ok(())
}
async fn finish(sftp: &Sftp, uploads: &Uploads, id: Uuid) -> Result<String, FsError> {
    let upload = get_upload(uploads, id).await?;
    let mut upload = upload.lock().await;
    if upload.failed {
        return Err(FsError::Other("failed upload cannot be published".into()));
    }
    if let Some(handle) = upload.handle.take() {
        sftp.raw
            .close(handle)
            .await
            .map_err(|e| fs_error(e, &upload.temporary))?;
    }
    let result = if upload.mode == UploadMode::Replace {
        let mut data = Vec::new();
        for path in [&upload.temporary, &upload.destination] {
            data.extend_from_slice(&(path.len() as u32).to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        match sftp.raw.extended("posix-rename@openssh.com", data).await {
            Ok(Packet::Status(status)) if status.status_code == StatusCode::Ok => Ok(()),
            Ok(Packet::Status(status)) => Err(SftpError::Status(status)),
            Ok(_) => Err(SftpError::UnexpectedPacket),
            Err(e) => Err(e),
        }
    } else {
        sftp.raw
            .rename(&upload.temporary, &upload.destination)
            .await
            .map(|_| ())
    };
    if let Err(error) = result {
        if upload.mode == UploadMode::Create && stat(sftp, &upload.destination).await?.is_some() {
            return Err(FsError::AlreadyExists {
                path: upload.destination.clone(),
            });
        }
        return Err(fs_error(error, &upload.destination));
    }
    uploads.lock().await.remove(&id);
    Ok(upload.destination.clone())
}
async fn cancel(sftp: &Sftp, uploads: &Uploads, id: Uuid) -> Result<(), FsError> {
    let Some(upload) = uploads.lock().await.remove(&id) else {
        return Ok(());
    };
    let mut upload = upload.lock().await;
    if let Some(handle) = upload.handle.take() {
        let _ = sftp.raw.close(handle).await;
    }
    sftp.raw
        .remove(&upload.temporary)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, &upload.temporary))
}
/// SFTP v3 cannot atomically express O_NOFOLLOW. Reject the final symlink
/// before open and verify the opened handle is regular; this still cannot
/// prove path identity against a concurrent remote rename/symlink swap.
async fn start_download(
    sftp: &Sftp,
    downloads: &Downloads,
    source: String,
    fs: SshFs,
) -> Result<Box<dyn RemoteDownload>, FsError> {
    let metadata = stat(sftp, &source)
        .await?
        .ok_or_else(|| FsError::NotFound {
            path: source.clone(),
        })?;
    if metadata.kind != EntryKind::File || metadata.is_symlink {
        return Err(FsError::Unsupported(format!(
            "{source}: only regular files can be downloaded"
        )));
    }
    let id = Uuid::new_v4();
    let download = Arc::new(Mutex::new(Download {
        source: source.clone(),
        handle: None,
        offset: 0,
        eof: false,
    }));
    {
        let mut registry = downloads.lock().await;
        if registry.len() >= MAX_OPEN_DOWNLOADS {
            return Err(FsError::Other(
                "too many open downloads; close an existing reader".into(),
            ));
        }
        registry.insert(id, download.clone());
    }
    // Register ownership before awaiting OPEN, so session shutdown owns any
    // handle acquired while a request is in progress.
    let result = async {
        let handle = sftp
            .raw
            .open(&source, OpenFlags::READ, FileAttributes::empty())
            .await
            .map_err(|error| fs_error(error, &source))?
            .handle;
        download.lock().await.handle = Some(handle.clone());
        let attrs = sftp
            .raw
            .fstat(&handle)
            .await
            .map_err(|error| fs_error(error, &source))?
            .attrs;
        if attrs.file_type() != FileType::File {
            return Err(FsError::Unsupported(format!(
                "{source}: opened object is not a regular file"
            )));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(Box::new(Reader { id, fs })),
        Err(error) => {
            let _ = close_download(sftp, downloads, id).await;
            Err(error)
        }
    }
}
async fn read(
    sftp: &Sftp,
    downloads: &Downloads,
    id: Uuid,
    max_bytes: usize,
) -> Result<Vec<u8>, FsError> {
    let max_bytes = max_bytes.min(MAX_CHUNK);
    if max_bytes == 0 {
        return Err(FsError::Other("download read size must be positive".into()));
    }
    let download = downloads
        .lock()
        .await
        .get(&id)
        .cloned()
        .ok_or(FsError::NotConnected)?;
    let mut download = download.lock().await;
    let handle = download.handle.as_ref().ok_or(FsError::NotConnected)?;
    if download.eof {
        return Ok(Vec::new());
    }
    match sftp
        .raw
        .read(handle, download.offset, max_bytes.min(MAX_CHUNK) as u32)
        .await
    {
        Ok(data) => {
            if data.data.is_empty() || data.data.len() > max_bytes {
                return Err(FsError::Other(format!(
                    "{}: invalid SFTP read length",
                    download.source
                )));
            }
            download.offset = download
                .offset
                .checked_add(data.data.len() as u64)
                .ok_or_else(|| FsError::Other("download offset overflow".into()))?;
            Ok(data.data)
        }
        Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
            download.eof = true;
            Ok(Vec::new())
        }
        Err(error) => Err(fs_error(error, &download.source)),
    }
}
async fn close_download(sftp: &Sftp, downloads: &Downloads, id: Uuid) -> Result<(), FsError> {
    let Some(download) = downloads.lock().await.remove(&id) else {
        return Ok(());
    };
    let mut download = download.lock().await;
    if let Some(handle) = download.handle.take() {
        sftp.raw
            .close(handle)
            .await
            .map_err(|error| fs_error(error, &download.source))?;
    }
    Ok(())
}

fn entry_kind(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::Dir => EntryKind::Directory,
        FileType::File => EntryKind::File,
        _ => EntryKind::Other,
    }
}
fn fs_error(error: SftpError, path: &str) -> FsError {
    let path = path.to_owned();
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::PermissionDenied => FsError::PermissionDenied { path },
            StatusCode::NoSuchFile => FsError::NotFound { path },
            _ => FsError::Other(format!("{path}: {}", status.error_message)),
        },
        other => FsError::Other(format!("{path}: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::time::{Duration, timeout};

    async fn packet(peer: &mut DuplexStream) -> Vec<u8> {
        let len = peer.read_u32().await.unwrap() as usize;
        assert!(len <= MAX_CHUNK + 128);
        let mut bytes = vec![0; len];
        peer.read_exact(&mut bytes).await.unwrap();
        bytes
    }
    async fn send(peer: &mut DuplexStream, bytes: &[u8]) {
        peer.write_u32(bytes.len() as u32).await.unwrap();
        peer.write_all(bytes).await.unwrap();
    }
    async fn status(peer: &mut DuplexStream, id: u32, code: StatusCode) {
        let mut bytes = vec![101]; // SSH_FXP_STATUS
        bytes.extend(id.to_be_bytes());
        bytes.extend((code as u32).to_be_bytes());
        bytes.extend([0; 8]); // empty error message and language tag
        send(peer, &bytes).await;
    }
    fn request_id(packet: &[u8]) -> u32 {
        u32::from_be_bytes(packet[1..5].try_into().unwrap())
    }

    #[tokio::test]
    async fn download_closes_handle_when_fstat_rejects_object_changed_after_lstat() {
        let (client, mut peer) = tokio::io::duplex(1024);
        let raw = RawSftpSession::new(client);
        let fake = tokio::spawn(async move {
            assert_eq!(packet(&mut peer).await[0], 1);
            send(&mut peer, &[2, 0, 0, 0, 3]).await;
            let lstat = packet(&mut peer).await;
            assert_eq!(lstat[0], 7);
            let mut attrs = vec![105]; // ATTRS: permissions only
            attrs.extend(request_id(&lstat).to_be_bytes());
            attrs.extend(4u32.to_be_bytes());
            attrs.extend(0o100600u32.to_be_bytes());
            send(&mut peer, &attrs).await;
            let open = packet(&mut peer).await;
            assert_eq!(open[0], 3);
            let mut handle = vec![102];
            handle.extend(request_id(&open).to_be_bytes());
            handle.extend(6u32.to_be_bytes());
            handle.extend(b"handle");
            send(&mut peer, &handle).await;
            let fstat = packet(&mut peer).await;
            assert_eq!(fstat[0], 8);
            attrs[1..5].copy_from_slice(&request_id(&fstat).to_be_bytes());
            attrs[9..13].copy_from_slice(&0o040755u32.to_be_bytes());
            send(&mut peer, &attrs).await;
            let close = packet(&mut peer).await;
            assert_eq!(close[0], 4);
            status(&mut peer, request_id(&close), StatusCode::Ok).await;
        });
        timeout(Duration::from_secs(3), raw.init())
            .await
            .unwrap()
            .unwrap();
        let sftp = Sftp {
            raw,
            atomic_replace: false,
        };
        let downloads: Downloads = Arc::default();
        let (fs, _requests) = channel();
        assert!(matches!(
            timeout(
                Duration::from_secs(3),
                start_download(&sftp, &downloads, "/source".into(), fs)
            )
            .await
            .unwrap(),
            Err(FsError::Unsupported(_))
        ));
        assert!(downloads.lock().await.is_empty());
        timeout(Duration::from_secs(3), fake)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn batch_sends_full_window_drains_failure_and_never_publishes_partial_file() {
        let (client, mut peer) = tokio::io::duplex(64 * 1024);
        let raw = RawSftpSession::new(client);
        let (failure_sent, failure_received) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let fake = tokio::spawn(async move {
            assert_eq!(packet(&mut peer).await[0], 1); // INIT
            send(&mut peer, &[2, 0, 0, 0, 3]).await; // VERSION 3
            let mut requests = Vec::new();
            let mut offset = 0;
            // Withhold every ACK until all eight WRITE packets arrive.
            for index in 0..MAX_WRITE_BATCH {
                let write = packet(&mut peer).await;
                assert_eq!(write[0], 6);
                let handle_len = u32::from_be_bytes(write[5..9].try_into().unwrap()) as usize;
                assert_eq!(&write[9..9 + handle_len], b"handle");
                let start = 9 + handle_len;
                assert_eq!(
                    u64::from_be_bytes(write[start..start + 8].try_into().unwrap()),
                    offset
                );
                let size =
                    u32::from_be_bytes(write[start + 8..start + 12].try_into().unwrap()) as usize;
                assert_eq!(size, MAX_CHUNK - index);
                assert_eq!(&write[start + 12..], vec![index as u8; size]);
                offset += size as u64;
                requests.push(request_id(&write));
            }
            status(&mut peer, requests[0], StatusCode::Failure).await;
            failure_sent.send(()).unwrap();
            resume.await.unwrap();
            // Replies can be reordered. Every remaining reply must be drained.
            for id in requests.into_iter().skip(1).rev() {
                status(&mut peer, id, StatusCode::Ok).await;
            }
            // After rejected finish, cancellation closes and removes only the
            // temporary file. A RENAME would fail these packet assertions.
            let close = packet(&mut peer).await;
            assert_eq!(close[0], 4);
            status(&mut peer, request_id(&close), StatusCode::Ok).await;
            let remove = packet(&mut peer).await;
            assert_eq!(remove[0], 13);
            assert_eq!(&remove[9..], b"/tmp/.partial");
            status(&mut peer, request_id(&remove), StatusCode::Ok).await;
        });
        timeout(Duration::from_secs(3), raw.init())
            .await
            .unwrap()
            .unwrap();
        let sftp = Arc::new(Sftp {
            raw,
            atomic_replace: false,
        });
        let uploads: Uploads = Arc::default();
        let id = Uuid::new_v4();
        uploads.lock().await.insert(
            id,
            Arc::new(Mutex::new(Upload {
                temporary: "/tmp/.partial".into(),
                destination: "/tmp/target".into(),
                handle: Some("handle".into()),
                offset: 0,
                mode: UploadMode::Create,
                failed: false,
            })),
        );
        let write = tokio::spawn({
            let sftp = sftp.clone();
            let uploads = uploads.clone();
            async move {
                write_batch(
                    &sftp,
                    &uploads,
                    id,
                    (0..MAX_WRITE_BATCH)
                        .map(|n| vec![n as u8; MAX_CHUNK - n])
                        .collect(),
                )
                .await
            }
        });
        timeout(Duration::from_secs(3), failure_received)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !write.is_finished(),
            "an early failed ACK must not abandon the remaining requests"
        );
        let publish = tokio::spawn({
            let sftp = sftp.clone();
            let uploads = uploads.clone();
            async move { finish(&sftp, &uploads, id).await }
        });
        assert!(!publish.is_finished());
        release.send(()).unwrap();
        assert!(
            timeout(Duration::from_secs(3), write)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(
            timeout(Duration::from_secs(3), publish)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        let upload = get_upload(&uploads, id).await.unwrap();
        assert!(upload.lock().await.failed);
        assert_eq!(upload.lock().await.offset, 0);
        assert!(
            write_batch(&sftp, &uploads, id, vec![b"retry".to_vec()])
                .await
                .is_err()
        );
        timeout(Duration::from_secs(3), cancel(&sftp, &uploads, id))
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(3), fake)
            .await
            .unwrap()
            .unwrap();
        assert!(uploads.lock().await.is_empty());
    }
}
