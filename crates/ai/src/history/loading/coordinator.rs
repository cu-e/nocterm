//! One prepaid outer native task retains the original identity through the history gate.
use super::{
    HistoryGate, HistoryPermit, ReaderDelivery, ReaderDocumentId, ReaderSource, ReaderSourceError,
};
use crate::{
    model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch},
    storage::ChatRepository,
};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

type LoadResult = Result<(ReaderDelivery, HistoryPermit), ReaderSourceError>;

/// The actual concrete future allocation retires before its exact original credit.
/// No Reader capability is captured until the existing shared gate has admitted it.
pub struct PreparedReaderLoad {
    work: Pin<Box<dyn Future<Output = LoadResult> + Send>>,
    _allocation: NativeScratch,
}
impl PreparedReaderLoad {
    pub(super) fn prepare(
        id: ReaderDocumentId,
        ledger: DecodeLedger,
        source: ReaderSource,
        gate: HistoryGate,
        repository: Arc<dyn ChatRepository>,
    ) -> Result<Self, NativeAllocationError> {
        let work = async move {
            let permit = gate.acquire().await;
            let request = source.capture()?;
            let delivery = match repository.load_managed(id, request) {
                Ok(operation) => operation.await,
                Err(refusal) => refusal.into_delivery(),
            };
            Ok((delivery, permit))
        };
        let allocation = NativeScratch::reserve_native(&ledger, std::mem::size_of_val(&work))?;
        Ok(Self {
            work: Box::pin(work),
            _allocation: allocation,
        })
    }
}
impl Future for PreparedReaderLoad {
    type Output = LoadResult;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<LoadResult> {
        self.get_mut().work.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests;
