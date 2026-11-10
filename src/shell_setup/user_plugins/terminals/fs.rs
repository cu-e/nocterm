//! The files of a plugin terminal: a [`RemoteFs`] whose every request the plugin answers.
//!
//! The file browser and an AI chat's file tools use it as they use a server's SFTP. Each
//! request becomes a `file-request` event for the plugin; its `terminals.files` effect is the
//! answer. Paths are checked before the plugin is asked, contents travel as base64 and are
//! bounded on both ways, and nothing here touches a file of this computer.

use futures::{
    SinkExt as _,
    channel::{mpsc, oneshot},
};
use nocterm_plugin_api::{
    base64,
    capability::files::{FileAnswer, FileEntry, FileFailure, FileOp, MAX_FILE_BYTES},
};
use nocterm_session::{
    DirEntry, EntryKind, FileMetadata, FsCapabilities, FsError, FsFuture, RemoteFs,
    fs::{RemoteDownload, RemoteUpload, UploadMode},
};

/// One request on its way to the plugin, with the place its answer goes.
pub(super) struct Asked {
    op: FileOp,
    reply: oneshot::Sender<FileAnswer>,
}

impl Asked {
    pub(super) fn into_parts(self) -> (FileOp, oneshot::Sender<FileAnswer>) {
        (self.op, self.reply)
    }
}

/// The file system of one plugin terminal. Cloning it shares the way to the plugin.
#[derive(Clone)]
pub(super) struct PluginFs {
    asks: mpsc::Sender<Asked>,
}

impl PluginFs {
    pub(super) fn new(asks: mpsc::Sender<Asked>) -> Self {
        Self { asks }
    }

    /// Asks the plugin and waits for its answer. A path the host would not ask about is refused
    /// here, and a closed tab or a plugin that never answers is an error.
    async fn ask(mut self, op: FileOp) -> Result<FileAnswer, FsError> {
        op.check().map_err(|error| {
            FsError::Other(format!("not a path the plugin is asked about: {error}"))
        })?;
        let (reply, answer) = oneshot::channel();
        self.asks
            .send(Asked { op, reply })
            .await
            .map_err(|_| FsError::NotConnected)?;
        answer
            .await
            .map_err(|_| FsError::Other("the plugin did not answer".into()))
    }

    fn request<T: Send + 'static>(
        &self,
        op: FileOp,
        path: &str,
        read: impl FnOnce(FileAnswer, &str) -> Result<T, FsError> + Send + 'static,
    ) -> FsFuture<T> {
        let (fs, path) = (self.clone(), path.to_owned());
        Box::pin(async move {
            match fs.ask(op).await? {
                FileAnswer::Failed(failure) => Err(failed(failure, &path)),
                answer => read(answer, &path),
            }
        })
    }

    fn done(&self, op: FileOp, path: &str) -> FsFuture<()> {
        self.request(op, path, |answer, _| match answer {
            FileAnswer::Done => Ok(()),
            _ => Err(unexpected()),
        })
    }
}

fn unexpected() -> FsError {
    FsError::Other("the plugin answered something else than was asked".into())
}

fn failed(failure: FileFailure, path: &str) -> FsError {
    let path = path.to_owned();
    match failure {
        FileFailure::NotFound => FsError::NotFound { path },
        FileFailure::PermissionDenied => FsError::PermissionDenied { path },
        FileFailure::AlreadyExists => FsError::AlreadyExists { path },
        FileFailure::Unsupported => FsError::Unsupported("this operation".into()),
        FileFailure::Other(reason) => FsError::Other(reason),
    }
}

fn entry(entry: FileEntry) -> DirEntry {
    DirEntry {
        kind: if entry.directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        name: entry.name,
        is_symlink: false,
        size: entry.size,
    }
}

impl RemoteFs for PluginFs {
    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            metadata: true,
            rename: true,
            remove: true,
            set_permissions: false,
        }
    }

    fn home(&self) -> FsFuture<String> {
        Box::pin(async { Ok(String::from("/")) })
    }

    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let op = FileOp::List {
            path: path.to_owned(),
        };
        self.request(op, path, |answer, _| match answer {
            FileAnswer::Listed(entries) => Ok(entries.into_iter().map(entry).collect()),
            _ => Err(unexpected()),
        })
    }

    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let op = FileOp::Stat {
            path: path.to_owned(),
        };
        self.request(op, path, |answer, _| match answer {
            FileAnswer::Found(found) => Ok(found.map(entry)),
            _ => Err(unexpected()),
        })
    }

    fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
        let op = FileOp::Stat {
            path: path.to_owned(),
        };
        self.request(op, path, |answer, path| match answer {
            FileAnswer::Found(Some(found)) => Ok(FileMetadata {
                kind: if found.directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                is_symlink: false,
                size: found.size,
                permissions: None,
                uid: None,
                gid: None,
                modified: found.modified,
            }),
            FileAnswer::Found(None) => Err(FsError::NotFound {
                path: path.to_owned(),
            }),
            _ => Err(unexpected()),
        })
    }

    /// Creates the directory, or accepts one that is there already.
    fn create_dir(&self, path: &str) -> FsFuture<()> {
        let (fs, owned) = (self.clone(), path.to_owned());
        Box::pin(async move {
            let op = FileOp::CreateDir {
                path: owned.clone(),
            };
            match fs.clone().ask(op).await? {
                FileAnswer::Done => Ok(()),
                FileAnswer::Failed(FileFailure::AlreadyExists) => {
                    match fs
                        .ask(FileOp::Stat {
                            path: owned.clone(),
                        })
                        .await?
                    {
                        FileAnswer::Found(Some(found)) if found.directory => Ok(()),
                        _ => Err(FsError::AlreadyExists { path: owned }),
                    }
                }
                FileAnswer::Failed(failure) => Err(failed(failure, &owned)),
                _ => Err(unexpected()),
            }
        })
    }

    fn remove_file(&self, path: &str) -> FsFuture<()> {
        let op = FileOp::Remove {
            path: path.to_owned(),
            directory: false,
        };
        self.done(op, path)
    }

    fn remove_dir(&self, path: &str) -> FsFuture<()> {
        let op = FileOp::Remove {
            path: path.to_owned(),
            directory: true,
        };
        self.done(op, path)
    }

    fn rename(&self, old: &str, new: &str) -> FsFuture<()> {
        let op = FileOp::Rename {
            from: old.to_owned(),
            to: new.to_owned(),
        };
        self.done(op, old)
    }

    /// Reads the whole file, at most [`MAX_FILE_BYTES`], and serves it from memory.
    fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        let op = FileOp::Read {
            path: path.to_owned(),
        };
        self.request(op, path, |answer, _| match answer {
            FileAnswer::Content(data) => {
                let bytes = base64::decode(&data).ok_or_else(unexpected)?;
                Ok(Box::new(Downloaded { bytes, at: 0 }) as Box<dyn RemoteDownload>)
            }
            _ => Err(unexpected()),
        })
    }

    /// Collects the file in memory, at most [`MAX_FILE_BYTES`], and writes it whole when it is
    /// finished: nothing reaches the plugin before.
    fn upload(&self, path: &str, mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        let upload = Upload {
            fs: self.clone(),
            path: path.to_owned(),
            create: mode == UploadMode::Create,
            bytes: Vec::new(),
            published: false,
        };
        Box::pin(async move { Ok(Box::new(upload) as Box<dyn RemoteUpload>) })
    }
}

/// A file read whole, handed out in the chunks the caller asks for.
struct Downloaded {
    bytes: Vec<u8>,
    at: usize,
}

impl RemoteDownload for Downloaded {
    fn read(&mut self, max_bytes: usize) -> FsFuture<Vec<u8>> {
        let end = self.bytes.len().min(self.at.saturating_add(max_bytes));
        let chunk = self.bytes[self.at..end].to_vec();
        self.at = end;
        Box::pin(async move { Ok(chunk) })
    }

    fn close(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

/// A file collected in memory and written whole when it is published.
struct Upload {
    fs: PluginFs,
    path: String,
    create: bool,
    bytes: Vec<u8>,
    published: bool,
}

impl Upload {
    fn publish(&mut self, path: String) -> FsFuture<String> {
        if self.published {
            return Box::pin(async { Err(FsError::Other("the upload was published".into())) });
        }
        self.published = true;
        let op = FileOp::Write {
            path: path.clone(),
            data: base64::encode(&self.bytes),
            create: self.create,
        };
        let fs = self.fs.clone();
        Box::pin(async move {
            match fs.ask(op).await? {
                FileAnswer::Done => Ok(path),
                FileAnswer::Failed(failure) => Err(failed(failure, &path)),
                _ => Err(unexpected()),
            }
        })
    }
}

impl RemoteUpload for Upload {
    fn write(&mut self, bytes: Vec<u8>) -> FsFuture<()> {
        if self.published {
            return Box::pin(async { Err(FsError::Other("the upload was published".into())) });
        }
        if self.bytes.len() + bytes.len() > MAX_FILE_BYTES {
            return Box::pin(async {
                Err(FsError::Other(format!(
                    "a plugin's files hold at most {} KiB",
                    MAX_FILE_BYTES / 1024
                )))
            });
        }
        self.bytes.extend_from_slice(&bytes);
        Box::pin(async { Ok(()) })
    }

    fn publish_as(&mut self, name: String) -> FsFuture<String> {
        if !nocterm_session::fs::valid_publication_name(&name) {
            return Box::pin(async { Err(FsError::Other("not a file name".into())) });
        }
        let parent = nocterm_session::fs::path::parent(&self.path).to_owned();
        let path = nocterm_session::fs::path::join(&parent, &name);
        self.publish(path)
    }

    fn finish(mut self: Box<Self>) -> FsFuture<String> {
        let path = self.path.clone();
        self.publish(path)
    }

    fn cancel(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests;
