//! Container-only filesystem commands and bounded streaming ownership.
use nocterm_session::fs::{RemoteDownload, RemoteUpload, UploadMode};
mod actor;
mod codec;
mod commands;
mod download;
mod handles;
mod scripts;
mod upload;

use actor::{Client, Request};
use commands::{FilesystemCommands, checked_path};
use handles::{Lease, Reader, Writer};
use nocterm_project_providers::ContainerLocation;
use nocterm_session::{
    DirEntry, FileMetadata, FsCapabilities, FsError, FsFuture, HostExec, RemoteFs,
};
use std::sync::Arc;

pub(crate) struct ContainerFilesystem {
    client: Client,
}
impl ContainerFilesystem {
    pub(crate) fn new(exec: Arc<dyn HostExec>, location: ContainerLocation) -> Self {
        Self {
            client: Client::start(FilesystemCommands::new(exec, location)),
        }
    }
    fn run(
        &self,
        script: &'static str,
        path: &str,
        mutation: bool,
        extra: Vec<String>,
        limit: usize,
    ) -> FsFuture<Vec<u8>> {
        let client = self.client.clone();
        let path = checked_path(path, mutation);
        Box::pin(async move {
            let mut args = vec![path?];
            args.extend(extra);
            handles::bytes(
                client
                    .call(Request::Run {
                        script,
                        args,
                        limit,
                    })
                    .await?,
            )
        })
    }
    fn mutate(&self, script: &'static str, path: &str, extra: Vec<String>) -> FsFuture<()> {
        let future = self.run(script, path, true, extra, 128);
        Box::pin(async move { future.await.map(|_| ()) })
    }
}
impl RemoteFs for ContainerFilesystem {
    fn capabilities(&self) -> FsCapabilities {
        let supported = self.client.supported();
        FsCapabilities {
            metadata: supported,
            rename: supported,
            remove: supported,
            set_permissions: supported,
        }
    }
    fn home(&self) -> FsFuture<String> {
        let future = self.run(scripts::HOME, "/", false, vec![], 4096);
        Box::pin(async move {
            let bytes = future.await?;
            let home = codec::text(&bytes)?;
            checked_path(home, false)
        })
    }
    fn read_dir(&self, path: &str) -> FsFuture<Vec<DirEntry>> {
        let future = self.run(scripts::LIST, path, false, vec![], 24 * 1024 * 1024);
        Box::pin(async move { codec::listing(&future.await?) })
    }
    fn metadata(&self, path: &str) -> FsFuture<FileMetadata> {
        let future = self.run(scripts::STAT, path, false, vec![], 1024);
        Box::pin(async move { codec::metadata(&future.await?) })
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let future = self.metadata(path);
        Box::pin(async move {
            match future.await {
                Ok(metadata) => Ok(Some(codec::entry(name, metadata))),
                Err(FsError::NotFound { .. }) => Ok(None),
                Err(error) => Err(error),
            }
        })
    }
    fn create_dir(&self, path: &str) -> FsFuture<()> {
        self.mutate(scripts::CREATE_DIR, path, vec![])
    }
    fn remove_file(&self, path: &str) -> FsFuture<()> {
        self.mutate(scripts::REMOVE_FILE, path, vec![])
    }
    fn remove_dir(&self, path: &str) -> FsFuture<()> {
        self.mutate(scripts::REMOVE_DIR, path, vec![])
    }
    fn set_permissions(&self, path: &str, mode: u32) -> FsFuture<()> {
        if mode > 0o7777 {
            return Box::pin(async { Err(FsError::Other("invalid permission bits".into())) });
        }
        self.mutate(scripts::PERMISSIONS, path, vec![format!("{mode:o}")])
    }
    fn rename(&self, old: &str, new: &str) -> FsFuture<()> {
        match checked_path(new, true) {
            Ok(new) => self.mutate(scripts::RENAME, old, vec![new]),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
    fn upload(&self, path: &str, mode: UploadMode) -> FsFuture<Box<dyn RemoteUpload>> {
        let client = self.client.clone();
        let path = checked_path(path, true);
        let lease = Lease::new(client.clone());
        Box::pin(async move {
            let id = handles::id(
                client
                    .call(Request::Upload {
                        path: path?,
                        mode,
                        alive: lease.alive(),
                    })
                    .await?,
            )?;
            Ok(Box::new(Writer::new(lease, id)) as Box<dyn RemoteUpload>)
        })
    }
    fn download(&self, path: &str) -> FsFuture<Box<dyn RemoteDownload>> {
        let client = self.client.clone();
        let path = checked_path(path, true);
        let lease = Lease::new(client.clone());
        Box::pin(async move {
            let id = handles::id(
                client
                    .call(Request::Download {
                        path: path?,
                        alive: lease.alive(),
                    })
                    .await?,
            )?;
            Ok(Box::new(Reader::new(lease, id)) as Box<dyn RemoteDownload>)
        })
    }
}
