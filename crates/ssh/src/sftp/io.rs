//! Upload and download handles that forward to the session's SFTP worker.
use super::*;
pub(super) struct Writer {
    pub(super) id: Uuid,
    pub(super) fs: SshFs,
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
pub(super) struct Reader {
    pub(super) id: Uuid,
    pub(super) fs: SshFs,
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
