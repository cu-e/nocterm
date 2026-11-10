//! Double-buffered remote reads with acknowledged destination writes.
use crate::{chunks, scheduler::Work};
use futures::{
    FutureExt as _,
    future::{BoxFuture, join},
};
use nocterm_session::fs::{RemoteDownload, RemoteUpload};
use nocterm_transfers::{StreamBudget, TransferCause, TransferEvent, TransferOperation};
use std::future::Future;

pub(crate) type Failure = (TransferOperation, TransferCause);

pub(crate) trait BatchWriter: Send {
    fn write_batch(
        &mut self,
        work: &Work,
        chunks: Vec<Vec<u8>>,
    ) -> impl Future<Output = Result<(), TransferCause>> + Send;
}
impl BatchWriter for dyn RemoteUpload {
    async fn write_batch(
        &mut self,
        work: &Work,
        chunks: Vec<Vec<u8>>,
    ) -> Result<(), TransferCause> {
        work.remote(RemoteUpload::write_batch(self, chunks)).await
    }
}

/// Holds at most the previous write batch and the next read batch. Both futures
/// finish before either failure is returned, so a local destination regains its
/// stage before its caller attempts cleanup.
pub(crate) fn copy<'a, W: BatchWriter + ?Sized + 'a>(
    work: &'a Work,
    expected: Option<u64>,
    reader: &'a mut dyn RemoteDownload,
    writer: &'a mut W,
) -> BoxFuture<'a, Result<(), Failure>> {
    async move {
        let limits = work.state().limits().stream();
        let mut budget = StreamBudget::new(limits, expected);
        let mut pending: Vec<Vec<u8>> = Vec::new();
        let mut eof = false;
        loop {
            let writing = std::mem::take(&mut pending);
            if eof && writing.is_empty() {
                break;
            }
            let wrote = chunks::total(&writing);
            let read = async {
                if eof {
                    Ok(Vec::new())
                } else {
                    work.remote(reader.read_batch(limits.chunk(), limits.acknowledgements()))
                        .await
                }
            };
            let write = async {
                if writing.is_empty() {
                    Ok(())
                } else {
                    writer.write_batch(work, writing).await
                }
            };
            let (read, written) = join(read, write).await;
            written.map_err(|cause| (TransferOperation::Write, cause))?;
            let read = read.map_err(|cause| (TransferOperation::Read, cause))?;
            if wrote > 0 {
                budget
                    .acknowledged(wrote)
                    .map_err(|cause| (TransferOperation::Write, cause))?;
                work.batch().event(TransferEvent::Acknowledged {
                    bytes: wrote as u64,
                });
            }
            if read.is_empty() {
                eof = true;
            } else {
                chunks::accept_batch(&budget, &read)
                    .map_err(|cause| (TransferOperation::Read, cause))?;
                pending = read;
            }
        }
        budget
            .finish()
            .map_err(|cause| (TransferOperation::Read, cause))
    }
    .boxed()
}
