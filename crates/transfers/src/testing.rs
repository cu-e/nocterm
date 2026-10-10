//! Explicitly driven transfer-port fake; it performs no discovery or streaming.
use crate::{
    DownloadRequest, Progress, ProgressTracker, Request, TransferError, TransferEvent, TransferId,
    TransferLimits, TransferService, UploadRequest,
};
use futures::future::BoxFuture;
use nocterm_session::RemoteFs;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error)]
pub enum FakeTransferError {
    #[error("No recorded transfer has this ID.")]
    Missing,
    #[error("The fake recording bound was reached.")]
    Full,
    #[error("The fake transfer control lock is poisoned.")]
    Poisoned,
}
struct Recording {
    request: Request,
    tracker: ProgressTracker,
}
#[derive(Default)]
struct Recordings {
    next: u64,
    entries: BTreeMap<TransferId, Recording>,
    stopping: bool,
}
/// Tests choose transitions explicitly, leaving scheduling and byte handling to
/// adapter tests rather than duplicating those implementations in a fake.
#[derive(Default)]
pub struct FakeTransfers {
    recordings: Mutex<Recordings>,
}
impl FakeTransfers {
    pub fn request(&self, id: TransferId) -> Result<Request, FakeTransferError> {
        let recordings = self
            .recordings
            .lock()
            .map_err(|_| FakeTransferError::Poisoned)?;
        recordings
            .entries
            .get(&id)
            .map(|entry| entry.request.clone())
            .ok_or(FakeTransferError::Missing)
    }
    pub fn advance(&self, id: TransferId, event: TransferEvent) -> Result<(), FakeTransferError> {
        let mut recordings = self
            .recordings
            .lock()
            .map_err(|_| FakeTransferError::Poisoned)?;
        recordings
            .entries
            .get_mut(&id)
            .ok_or(FakeTransferError::Missing)?
            .tracker
            .apply(event);
        Ok(())
    }
    fn admit(&self, request: Request) -> Result<TransferId, TransferError> {
        let limits = TransferLimits::default();
        request.validate(limits.discovery())?;
        let mut recordings = self
            .recordings
            .lock()
            .map_err(|_| TransferError::Runtime(Arc::new(FakeTransferError::Poisoned)))?;
        if recordings.stopping {
            return Err(TransferError::ShuttingDown);
        }
        if recordings.entries.len() >= 256 {
            return Err(TransferError::Full);
        }
        let next = recordings
            .next
            .checked_add(1)
            .ok_or(TransferError::SequenceExhausted)?;
        let mut bytes = [0; 16];
        bytes[8..].copy_from_slice(&next.to_be_bytes());
        let id = TransferId::from_bytes(bytes)?;
        let tracker = ProgressTracker::new(id, &request, limits.stream());
        recordings
            .entries
            .insert(id, Recording { request, tracker });
        recordings.next = next;
        Ok(id)
    }
}
impl TransferService for FakeTransfers {
    fn enqueue(&self, request: UploadRequest) -> Result<TransferId, TransferError> {
        self.admit(Request::Upload(request))
    }
    fn enqueue_download(&self, request: DownloadRequest) -> Result<TransferId, TransferError> {
        self.admit(Request::Download(request))
    }
    fn snapshot(&self) -> Vec<Progress> {
        self.recordings
            .lock()
            .map(|recordings| {
                recordings
                    .entries
                    .values()
                    .rev()
                    .map(|entry| entry.tracker.snapshot())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn cancel(&self, id: TransferId) -> Result<(), TransferError> {
        for event in [
            TransferEvent::CancelRequested,
            TransferEvent::DiscoveryComplete,
            TransferEvent::WorkersDrained,
        ] {
            self.advance(id, event).map_err(|error| match error {
                FakeTransferError::Missing => TransferError::Missing,
                error => TransferError::Runtime(Arc::new(error)),
            })?;
        }
        Ok(())
    }
    fn retry(&self, id: TransferId, fs: Arc<dyn RemoteFs>) -> Result<TransferId, TransferError> {
        let request = {
            let recordings = self
                .recordings
                .lock()
                .map_err(|_| TransferError::Runtime(Arc::new(FakeTransferError::Poisoned)))?;
            let entry = recordings.entries.get(&id).ok_or(TransferError::Missing)?;
            if !entry.tracker.snapshot().state.finished() {
                return Err(TransferError::NotRetryable);
            }
            entry.request.clone().with_fs(fs)
        };
        self.admit(request)
    }
    fn freeze_and_drain(&self) -> BoxFuture<'static, Result<(), TransferError>> {
        let result = self
            .recordings
            .lock()
            .map_err(|_| TransferError::Runtime(Arc::new(FakeTransferError::Poisoned)))
            .map(|mut recordings| {
                recordings.stopping = true;
                for entry in recordings.entries.values_mut() {
                    for event in [
                        TransferEvent::CancelRequested,
                        TransferEvent::DiscoveryComplete,
                        TransferEvent::WorkersDrained,
                    ] {
                        entry.tracker.apply(event);
                    }
                }
            });
        Box::pin(async move { result })
    }
}
