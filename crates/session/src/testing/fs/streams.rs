use super::{FsOperation, Shared, lock};
use crate::{
    FsError, FsFuture,
    fs::{RemoteDownload, RemoteUpload, UploadMode},
};
use std::sync::{Arc, Mutex};

struct StagedBytes {
    bytes: Vec<u8>,
    closed: bool,
    published: bool,
    canceled: bool,
    failed: bool,
}
struct Stage {
    shared: Arc<Shared>,
    path: String,
    mode: UploadMode,
    data: Mutex<StagedBytes>,
}
impl Drop for Stage {
    fn drop(&mut self) {
        let mut activity = lock(&self.shared.activity);
        activity.live_uploads = activity.live_uploads.saturating_sub(1);
        let data = lock(&self.data);
        if !data.published && !data.canceled {
            activity.abandoned_uploads = activity.abandoned_uploads.saturating_add(1);
        }
    }
}

pub(super) struct Upload {
    stage: Arc<Stage>,
}
impl Upload {
    pub(super) fn new(shared: Arc<Shared>, path: String, mode: UploadMode) -> Self {
        let mut activity = lock(&shared.activity);
        activity.live_uploads = activity.live_uploads.saturating_add(1);
        activity.peak_uploads = activity.peak_uploads.max(activity.live_uploads);
        drop(activity);
        Self {
            stage: Arc::new(Stage {
                shared,
                path,
                mode,
                data: Mutex::new(StagedBytes {
                    bytes: Vec::new(),
                    closed: false,
                    published: false,
                    canceled: false,
                    failed: false,
                }),
            }),
        }
    }

    fn append(&self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        let stage = self.stage.clone();
        Box::pin(async move {
            {
                let mut data = lock(&stage.data);
                if data.closed || data.failed {
                    return Err(FsError::NotConnected);
                }
                data.failed = true;
            }
            if let Err(error) = stage.shared.before(FsOperation::Write, &stage.path).await {
                lock(&stage.data).failed = true;
                return Err(error);
            }
            let limits = lock(&stage.shared.limits);
            if chunks.len() > 8 || chunks.iter().any(|chunk| chunk.len() > limits.chunk) {
                lock(&stage.data).failed = true;
                return Err(FsError::Unsupported("upload chunk or batch bound".into()));
            }
            let mut data = lock(&stage.data);
            if data.closed {
                return Err(FsError::NotConnected);
            }
            let size = chunks
                .iter()
                .try_fold(data.bytes.len(), |size, chunk| {
                    size.checked_add(chunk.len())
                })
                .filter(|size| *size <= limits.file)
                .ok_or_else(|| {
                    data.failed = true;
                    FsError::Unsupported("fake file size bound".into())
                })?;
            let additional = size - data.bytes.len();
            {
                let mut activity = lock(&stage.shared.activity);
                activity.max_write_chunk = activity
                    .max_write_chunk
                    .max(chunks.iter().map(Vec::len).max().unwrap_or(0));
                activity.max_write_batch = activity.max_write_batch.max(chunks.len());
                activity.acknowledged_bytes =
                    activity.acknowledged_bytes.saturating_add(additional);
            }
            data.bytes.reserve(additional);
            for chunk in chunks {
                data.bytes.extend(chunk);
            }
            data.failed = false;
            Ok(())
        })
    }
}
impl RemoteUpload for Upload {
    fn write(&mut self, bytes: Vec<u8>) -> FsFuture<()> {
        self.append(vec![bytes])
    }
    fn write_batch(&mut self, chunks: Vec<Vec<u8>>) -> FsFuture<()> {
        self.append(chunks)
    }
    fn publish_as(&mut self, name: String) -> FsFuture<String> {
        publish(self.stage.clone(), name)
    }
    fn finish(self: Box<Self>) -> FsFuture<String> {
        let name = crate::fs::path::file_name(&self.stage.path).to_owned();
        publish(self.stage.clone(), name)
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            let stage = &self.stage;
            stage
                .shared
                .before(FsOperation::Cancel, &stage.path)
                .await?;
            let mut data = lock(&stage.data);
            if data.published {
                return Ok(());
            }
            data.closed = true;
            data.canceled = true;
            data.bytes.clear();
            let mut activity = lock(&stage.shared.activity);
            activity.canceled_uploads = activity.canceled_uploads.saturating_add(1);
            Ok(())
        })
    }
}

fn publish(stage: Arc<Stage>, name: String) -> FsFuture<String> {
    Box::pin(async move {
        if !crate::fs::valid_publication_name(&name) {
            return Err(FsError::Other("invalid publication leaf name".into()));
        }
        let destination = crate::fs::path::join(crate::fs::path::parent(&stage.path), &name);
        {
            let mut data = lock(&stage.data);
            if data.failed || data.published || data.canceled {
                return Err(FsError::NotConnected);
            }
            data.closed = true;
            // A cancelled publication has no acknowledged outcome. Seal before
            // suspension, and only a known result can authorize another retry.
            data.failed = true;
        }
        stage
            .shared
            .before(FsOperation::Finish, &destination)
            .await?;
        let mut data = lock(&stage.data);
        if data.published || data.canceled {
            return Err(FsError::NotConnected);
        }
        // Collision probing precedes moving bytes: the same sealed stage remains
        // available, and neither the existing file nor its bytes are changed.
        let mut tree = lock(&stage.shared.tree);
        if let Err(error) = tree.upload_path(&destination, stage.mode) {
            data.failed = !matches!(error, FsError::AlreadyExists { .. });
            return Err(error);
        }
        if let Err(error) = tree.publish(&destination, stage.mode, std::mem::take(&mut data.bytes))
        {
            data.failed = true;
            return Err(error);
        }
        data.failed = false;
        data.published = true;
        lock(&stage.shared.activity).completed_uploads += 1;
        Ok(destination)
    })
}

struct Position {
    offset: usize,
    closed: bool,
}
struct ReadHandle {
    shared: Arc<Shared>,
    path: String,
    bytes: Vec<u8>,
    position: Mutex<Position>,
}
impl Drop for ReadHandle {
    fn drop(&mut self) {
        let mut activity = lock(&self.shared.activity);
        activity.live_downloads = activity.live_downloads.saturating_sub(1);
    }
}
pub(super) struct Reader {
    handle: Arc<ReadHandle>,
}
impl Reader {
    pub(super) fn new(shared: Arc<Shared>, path: String, bytes: Vec<u8>) -> Self {
        let mut activity = lock(&shared.activity);
        activity.live_downloads = activity.live_downloads.saturating_add(1);
        activity.peak_downloads = activity.peak_downloads.max(activity.live_downloads);
        drop(activity);
        Self {
            handle: Arc::new(ReadHandle {
                shared,
                path,
                bytes,
                position: Mutex::new(Position {
                    offset: 0,
                    closed: false,
                }),
            }),
        }
    }
}
impl RemoteDownload for Reader {
    fn read(&mut self, max_bytes: usize) -> FsFuture<Vec<u8>> {
        let handle = self.handle.clone();
        Box::pin(async move {
            {
                let mut activity = lock(&handle.shared.activity);
                activity.max_read_request = activity.max_read_request.max(max_bytes);
            }
            handle
                .shared
                .before(FsOperation::Read, &handle.path)
                .await?;
            if max_bytes == 0 {
                return Err(FsError::Unsupported("zero read bound".into()));
            }
            let response = {
                let mut reads = lock(&handle.shared.scripted_reads);
                reads
                    .iter()
                    .position(|(path, _)| path == &handle.path)
                    .and_then(|index| reads.remove(index))
                    .map(|(_, bytes)| bytes)
            };
            if let Some(bytes) = response {
                return Ok(bytes);
            }
            let mut position = lock(&handle.position);
            if position.closed {
                return Err(FsError::NotConnected);
            }
            let bound = max_bytes.min(lock(&handle.shared.limits).chunk);
            let end = position
                .offset
                .saturating_add(bound)
                .min(handle.bytes.len());
            let bytes = handle.bytes[position.offset..end].to_vec();
            position.offset = end;
            Ok(bytes)
        })
    }
    fn close(self: Box<Self>) -> FsFuture<()> {
        Box::pin(async move {
            let handle = &self.handle;
            handle
                .shared
                .before(FsOperation::Close, &handle.path)
                .await?;
            lock(&handle.position).closed = true;
            let mut activity = lock(&handle.shared.activity);
            activity.closed_downloads = activity.closed_downloads.saturating_add(1);
            Ok(())
        })
    }
}
