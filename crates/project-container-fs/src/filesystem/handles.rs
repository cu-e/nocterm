use super::actor::{Client, Request, Response};
use nocterm_session::fs::{RemoteDownload, RemoteUpload};
use nocterm_session::{FsError, FsFuture};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Lease {
    client: Client,
    alive: Arc<AtomicBool>,
}
impl Lease {
    pub(super) fn new(client: Client) -> Self {
        Self {
            client,
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
    pub(super) fn alive(&self) -> Arc<AtomicBool> {
        self.alive.clone()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.client.wake();
    }
}
pub(super) struct Writer {
    lease: Lease,
    id: u64,
    poisoned: bool,
}
impl Writer {
    pub(super) fn new(lease: Lease, id: u64) -> Self {
        Self {
            lease,
            id,
            poisoned: false,
        }
    }
}
impl RemoteUpload for Writer {
    fn write(&mut self, chunk: Vec<u8>) -> FsFuture<()> {
        self.write_batch(vec![chunk])
    }
    fn write_batch(&mut self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        if self.poisoned || chunks.len() > 8 || chunks.iter().any(|chunk| chunk.len() > 32768) {
            self.poisoned = true;
            return Box::pin(async {
                Err(FsError::Other(
                    "upload chunk bounds exceeded or upload failed".into(),
                ))
            });
        }
        let client = self.lease.client.clone();
        let id = self.id;
        Box::pin(async move { client.call(Request::Write(id, chunks)).await.map(|_| ()) })
    }
    fn publish_as(&mut self, name: String) -> FsFuture<String> {
        let client = self.lease.client.clone();
        let id = self.id;
        let poisoned = self.poisoned;
        Box::pin(async move {
            if poisoned {
                return Err(FsError::Other("failed upload cannot publish".into()));
            }
            path(client.call(Request::Publish(id, Some(name))).await?)
        })
    }
    fn finish(self: Box<Self>) -> FsFuture<String> {
        Box::pin(async move {
            if self.poisoned {
                return Err(FsError::Other("failed upload cannot publish".into()));
            }
            path(
                self.lease
                    .client
                    .call(Request::Publish(self.id, None))
                    .await?,
            )
        })
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            self.lease
                .client
                .call(Request::Cancel(self.id))
                .await
                .map(|_| ())
        })
    }
}
pub(super) struct Reader {
    lease: Lease,
    id: u64,
}
impl Reader {
    pub(super) fn new(lease: Lease, id: u64) -> Self {
        Self { lease, id }
    }
}
impl RemoteDownload for Reader {
    fn read(&mut self, max: usize) -> FsFuture<Vec<u8>> {
        let client = self.lease.client.clone();
        let id = self.id;
        Box::pin(async move { bytes(client.call(Request::Read(id, max)).await?) })
    }
    fn close(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            self.lease
                .client
                .call(Request::Close(self.id))
                .await
                .map(|_| ())
        })
    }
}
pub(super) fn bytes(response: Response) -> Result<Vec<u8>, FsError> {
    match response {
        Response::Bytes(bytes) => Ok(bytes),
        _ => Err(FsError::Other("unexpected filesystem response".into())),
    }
}
fn path(response: Response) -> Result<String, FsError> {
    match response {
        Response::Path(path) => Ok(path),
        _ => Err(FsError::Other("unexpected filesystem response".into())),
    }
}
pub(super) fn id(response: Response) -> Result<u64, FsError> {
    match response {
        Response::Id(id) => Ok(id),
        _ => Err(FsError::Other("unexpected filesystem response".into())),
    }
}
