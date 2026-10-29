//! One service pipeline owns native preflight, shared admission and actual workers.
use super::HistoryReader;
use crate::history_admission::{HistoryAdmissionHandle, HistoryPayloadPermit};
use futures::channel::oneshot;
use gpui::{App, BackgroundExecutor, Task};
use nocterm_ai::{
    history::{HistoryPermit, SavedChat},
    storage::AgentStorageError,
};
use nocterm_core::allocation::{Allocated, AllocationLease};

pub(super) struct AdmittedHistoryLoad {
    reader: HistoryReader,
    id: String,
    admission: HistoryAdmissionHandle,
}
/// Decoded data, its actual allocation, and raw-history delivery all retire together.
/// Consumers may inspect the borrowed body or atomically move it to its new owner.
pub struct LoadedHistory {
    result: Result<Allocated<SavedChat>, AgentStorageError>,
    payload: Option<HistoryPayloadPermit>,
    permit: Option<HistoryPermit>,
}
impl LoadedHistory {
    pub fn value(&self) -> Result<&SavedChat, &AgentStorageError> {
        self.result.as_ref().map(Allocated::value)
    }
    /// The callback inspects/drops the body or adopts its same payload proof in
    /// a registered resident owner before returning. No naked body is returned.
    pub fn publish(
        self,
        publish: impl FnOnce(
            Result<SavedChat, AgentStorageError>,
            Option<HistoryPermit>,
            Option<HistoryPayloadPermit>,
        ),
    ) {
        match self.result {
            Ok(charged) => {
                charged.map(|chat| publish(Ok(chat), self.permit, self.payload));
            }
            Err(error) => publish(Err(error), self.permit, self.payload),
        }
    }
    fn failed(error: AgentStorageError, permit: Option<HistoryPermit>) -> Self {
        Self {
            result: Err(error),
            payload: None,
            permit,
        }
    }
}
impl AdmittedHistoryLoad {
    pub(super) fn new(
        reader: HistoryReader,
        id: String,
        admission: HistoryAdmissionHandle,
    ) -> Self {
        Self {
            reader,
            id,
            admission,
        }
    }
    pub(super) fn start(self, cx: &mut App) -> Task<LoadedHistory> {
        let prepared = detached(cx.background_executor(), async move {
            self.reader.prepare_load(self.id).await
        });
        cx.spawn(async move |cx| {
            let Ok((prepared, permit)) = prepared.await else {
                return LoadedHistory::failed(AgentStorageError::Interrupted, None);
            };
            let prepared = match prepared {
                Ok(value) => value,
                Err(error) => return LoadedHistory::failed(error, Some(permit)),
            };
            let allocation =
                cx.update(|cx| allocate(&self.admission, prepared.resident_bytes(), cx));
            let (allocation, payload) = match allocation {
                Ok(value) => value,
                Err(error) => return LoadedHistory::failed(error, Some(permit)),
            };
            let decoded = detached(cx.background_executor(), async move {
                (prepared.decode(allocation).await, payload, permit)
            });
            match decoded.await {
                Ok((result, payload, permit)) => LoadedHistory {
                    result,
                    payload: Some(payload),
                    permit: Some(permit),
                },
                Err(_) => LoadedHistory::failed(AgentStorageError::Interrupted, None),
            }
        })
    }
}
fn allocate(
    admission: &HistoryAdmissionHandle,
    bytes: usize,
    cx: &mut App,
) -> Result<(AllocationLease, HistoryPayloadPermit), AgentStorageError> {
    let payload = admission
        .reserve_payload_reclaiming(None, cx)
        .map_err(|error| error.into_storage())?;
    let proof = admission
        .reserve_unowned_bytes(bytes, cx)
        .map_err(|error| error.into_storage())?;
    Ok((
        AllocationLease::new(bytes, 1, (proof, payload.clone())),
        payload,
    ))
}
fn detached<T: Send + 'static>(
    executor: &BackgroundExecutor,
    work: impl Future<Output = T> + Send + 'static,
) -> oneshot::Receiver<T> {
    let (send, receive) = oneshot::channel();
    executor
        .spawn(async move {
            let _ = send.send(work.await);
        })
        .detach();
    receive
}
