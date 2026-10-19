//! One original reader loan surrounds the actual paid native materializer Box.
use super::{ReaderDelivery, ReaderRequest};
use crate::{
    history::NativeMaterialized,
    model::{
        json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch},
        native::NativeOwned,
    },
    storage::AgentStorageError,
};
use std::{
    fmt,
    future::Future,
    mem::size_of,
    pin::Pin,
    task::{Context, Poll},
};

/// An original Foreground-paid copy survives the shared history gate wait.
/// Its bytes and real credit cannot be detached or cloned through this port.
pub struct ReaderDocumentId(NativeOwned<String>);
impl ReaderDocumentId {
    pub(in crate::history) fn from_native(id: NativeOwned<String>) -> Self {
        Self(id)
    }
    pub fn prepare_load(
        self,
        source: crate::history::ReaderSource,
        gate: crate::history::HistoryGate,
        repository: std::sync::Arc<dyn crate::storage::ChatRepository>,
    ) -> Result<crate::history::PreparedReaderLoad, NativeAllocationError> {
        let ledger = self.0.ledger().share();
        crate::history::loading::coordinator::PreparedReaderLoad::prepare(
            self, ledger, source, gate, repository,
        )
    }
    pub fn as_str(&self) -> &str {
        self.0.get().as_str()
    }
}
/// Native implementations perform synchronous I/O only when this task is polled.
/// Domain owns this lifecycle, while adapters own filesystem behavior and grammar.
pub trait ReaderMaterializer: Send + 'static {
    /// A native immediate task is ready by default. Test adapters may retain
    /// their original scripted receiver here, without boxing another future.
    fn poll_ready(
        &mut self,
        _: &mut Context<'_>,
        _: &DecodeLedger,
    ) -> Poll<Result<(), AgentStorageError>> {
        Poll::Ready(Ok(()))
    }
    fn materialize(
        &mut self,
        ledger: &DecodeLedger,
    ) -> Result<NativeMaterialized, AgentStorageError>;
}
struct OwnedReadTask {
    task: Box<dyn ReaderMaterializer>,
    _allocation: NativeScratch,
}
struct PendingTask {
    task: OwnedReadTask,
    request: ReaderRequest,
}
/// A by-value operation; callers must not add an unpaid compatibility Future Box.
pub struct ManagedReaderFuture {
    /// `None` once the original request has completed.
    state: Option<PendingTask>,
}
impl Future for ManagedReaderFuture {
    type Output = ReaderDelivery;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        // Poll in place: a panic still drops the native task/Box/credit before
        // the original request field can recycle its reader capability.
        let readiness = match &mut this.state {
            Some(PendingTask { task, request }) => request
                .phase
                .as_ref()
                .expect("original reader phase")
                .stage_native(|ledger| task.task.poll_ready(cx, ledger)),
            None => return Poll::Pending,
        };
        let Poll::Ready(readiness) = readiness else {
            return Poll::Pending;
        };
        let Some(PendingTask { mut task, request }) = this.state.take() else {
            return Poll::Pending;
        };
        if let Err(error) = readiness {
            drop(task);
            return Poll::Ready(request.fail(error));
        }
        Poll::Ready(request.read(move |ledger| {
            let result = task.task.materialize(ledger);
            // Physical native task/captures, then Box, then its credit, all
            // precede the original phase's publication/finish/recycling.
            drop(task);
            result
        }))
    }
}
/// A refusal retains the original request rather than allocating a ready future.
/// Pending captures/errors physically retire before that request can recycle.
pub struct ReaderPreparationFailure {
    error: AgentStorageError,
    /// Declared before the request so it retires before the request recycles.
    id: Option<ReaderDocumentId>,
    request: ReaderRequest,
}
impl ReaderPreparationFailure {
    fn new(id: Option<ReaderDocumentId>, error: AgentStorageError, request: ReaderRequest) -> Self {
        Self { error, id, request }
    }
    pub fn error(&self) -> &AgentStorageError {
        &self.error
    }
    pub fn into_delivery(self) -> ReaderDelivery {
        let Self { error, id, request } = self;
        drop(id);
        request.fail(error)
    }
}
impl fmt::Debug for ReaderPreparationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReaderPreparationFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[expect(
    clippy::result_large_err,
    reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
)]
pub(super) fn prepare<T: ReaderMaterializer>(
    request: ReaderRequest,
    id: ReaderDocumentId,
    factory: impl FnOnce(ReaderDocumentId, &DecodeLedger) -> Result<T, AgentStorageError>,
) -> Result<ManagedReaderFuture, ReaderPreparationFailure> {
    let mut id = Some(id);
    let scope = &request.source.0.get().scope;
    if !id
        .as_ref()
        .expect("pending reader identity")
        .0
        .belongs_to_document(scope)
    {
        return Err(ReaderPreparationFailure::new(
            id,
            AgentStorageError::ManagedAdmission(NativeAllocationError::Retention(
                crate::model::retention::RetentionError::ForeignAuthority,
            )),
            request,
        ));
    }
    let task = request
        .phase
        .as_ref()
        .expect("original reader phase")
        .stage_native(|ledger| {
            let allocation = NativeScratch::reserve_native(ledger, size_of::<T>())
                .map_err(AgentStorageError::ManagedAdmission)?;
            let task = factory(id.take().expect("unique reader identity"), ledger)?;
            Ok(OwnedReadTask {
                task: Box::new(task),
                _allocation: allocation,
            })
        });
    match task {
        Ok(task) => Ok(ManagedReaderFuture {
            state: Some(PendingTask { task, request }),
        }),
        Err(error) => Err(ReaderPreparationFailure::new(id, error, request)),
    }
}

#[cfg(test)]
mod tests;
