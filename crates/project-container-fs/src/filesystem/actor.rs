use super::{commands::FilesystemCommands, download::Download, scripts, upload::Upload};
use futures::{
    channel::oneshot,
    future::{Either, select},
    pin_mut,
};
use nocterm_session::FsError;
use nocterm_session::fs::UploadMode;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

pub(super) enum Request {
    Run {
        script: &'static str,
        args: Vec<String>,
        limit: usize,
    },
    Upload {
        path: String,
        mode: UploadMode,
        alive: Arc<AtomicBool>,
    },
    Download {
        path: String,
        alive: Arc<AtomicBool>,
    },
    Write(u64, Vec<Vec<u8>>),
    Publish(u64, Option<String>),
    Cancel(u64),
    Read(u64, usize),
    Close(u64),
    Wake,
}
pub(super) enum Response {
    Bytes(Vec<u8>),
    Id(u64),
    Path(String),
    Unit,
}
pub(super) struct Job {
    pub(super) request: Request,
    pub(super) reply: oneshot::Sender<Result<Response, FsError>>,
}
#[derive(Clone)]
pub(super) struct Client {
    sender: async_channel::Sender<Job>,
    supported: Arc<AtomicU8>,
}
impl Client {
    pub(super) fn start(commands: FilesystemCommands) -> Self {
        let (sender, receiver) = async_channel::bounded::<Job>(32);
        let supported = Arc::new(AtomicU8::new(0));
        let status = supported.clone();
        let worker = Worker {
            commands,
            uploads: HashMap::new(),
            downloads: HashMap::new(),
            next: 0,
            supported: status,
        };
        std::thread::Builder::new()
            .name("container-files".into())
            .spawn(move || futures::executor::block_on(worker.run(receiver)))
            .ok();
        Self { sender, supported }
    }
    pub(super) fn supported(&self) -> bool {
        self.supported.load(Ordering::Acquire) == 1
    }
    pub(super) async fn call(&self, request: Request) -> Result<Response, FsError> {
        let (reply, result) = oneshot::channel();
        self.sender
            .send(Job { request, reply })
            .await
            .map_err(|_| FsError::NotConnected)?;
        result.await.map_err(|_| FsError::NotConnected)?
    }
    pub(super) fn wake(&self) {
        let (reply, _) = oneshot::channel();
        let _ = self.sender.try_send(Job {
            request: Request::Wake,
            reply,
        });
    }
}
struct Worker {
    commands: FilesystemCommands,
    uploads: HashMap<u64, Upload>,
    downloads: HashMap<u64, Download>,
    next: u64,
    supported: Arc<AtomicU8>,
}
impl Worker {
    async fn run(mut self, receiver: async_channel::Receiver<Job>) {
        while let Ok(mut job) = receiver.recv().await {
            self.reap(false).await;
            if job.reply.is_canceled() || matches!(job.request, Request::Wake) {
                continue;
            }
            let write_id = match job.request {
                Request::Write(id, _) => Some(id),
                _ => None,
            };
            let result = {
                let work = self.dispatch(job.request);
                let cancelled = job.reply.cancellation();
                pin_mut!(work, cancelled);
                match select(work, cancelled).await {
                    Either::Left((result, _)) => Some(result),
                    Either::Right(_) => None,
                }
            };
            if let Some(result) = result {
                if job.reply.send(result).is_err() {
                    self.poison(write_id);
                }
            } else {
                self.poison(write_id);
            }
        }
        self.reap(true).await;
    }
    fn poison(&mut self, id: Option<u64>) {
        if let Some(upload) = id.and_then(|id| self.uploads.get_mut(&id)) {
            upload.poison();
        }
    }
    async fn reap(&mut self, all: bool) {
        let expired: Vec<_> = self
            .uploads
            .iter()
            .filter(|(_, upload)| all || !upload.alive())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(upload) = self.uploads.remove(&id) {
                let _ = upload.cleanup(&self.commands).await;
            }
        }
        self.downloads
            .retain(|_, download| !all && download.alive());
    }
    async fn ready(&self) -> Result<(), FsError> {
        match self.supported.load(Ordering::Acquire) {
            1 => return Ok(()),
            2 => {
                return Err(FsError::Unsupported(
                    "container filesystem utilities unavailable".into(),
                ));
            }
            _ => {}
        }
        match self.commands.run(scripts::PROBE, &[], None, 128).await {
            Ok(bytes) if bytes == b"nocterm-fs-v1" => {
                self.supported.store(1, Ordering::Release);
                Ok(())
            }
            Ok(_) => Err(FsError::Unsupported(
                "container filesystem probe malformed".into(),
            )),
            Err(error) => {
                if matches!(error, FsError::Unsupported(_)) {
                    self.supported.store(2, Ordering::Release);
                }
                Err(error)
            }
        }
    }
    async fn dispatch(&mut self, request: Request) -> Result<Response, FsError> {
        self.ready().await?;
        match request {
            Request::Run {
                script,
                args,
                limit,
            } => self
                .commands
                .run(script, &args, None, limit)
                .await
                .map(Response::Bytes),
            Request::Upload { path, mode, alive } => {
                self.capacity()?;
                let id = self.allocate();
                let upload = Upload::open(&self.commands, path, mode, alive).await?;
                self.uploads.insert(id, upload);
                Ok(Response::Id(id))
            }
            Request::Download { path, alive } => {
                self.capacity()?;
                let id = self.allocate();
                let download = Download::open(&self.commands, path, alive).await?;
                self.downloads.insert(id, download);
                Ok(Response::Id(id))
            }
            Request::Write(id, chunks) => {
                self.uploads
                    .get_mut(&id)
                    .ok_or(FsError::NotConnected)?
                    .write(&self.commands, chunks)
                    .await?;
                Ok(Response::Unit)
            }
            Request::Publish(id, name) => {
                let path = self
                    .uploads
                    .get_mut(&id)
                    .ok_or(FsError::NotConnected)?
                    .publish(&self.commands, name)
                    .await?;
                self.uploads.remove(&id);
                Ok(Response::Path(path))
            }
            Request::Cancel(id) => {
                if let Some(upload) = self.uploads.remove(&id) {
                    upload.cleanup(&self.commands).await?;
                }
                Ok(Response::Unit)
            }
            Request::Read(id, max) => self
                .downloads
                .get_mut(&id)
                .ok_or(FsError::NotConnected)?
                .read(max)
                .await
                .map(Response::Bytes),
            Request::Close(id) => {
                self.downloads.remove(&id);
                Ok(Response::Unit)
            }
            Request::Wake => Ok(Response::Unit),
        }
    }
    fn capacity(&self) -> Result<(), FsError> {
        if self.uploads.len() + self.downloads.len() >= 16 {
            Err(FsError::Other(
                "container filesystem stream limit reached".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn allocate(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}
