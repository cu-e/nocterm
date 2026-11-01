//! Streams one file from a source filesystem into a staged destination upload.
//!
//! The next read batch is in flight while the previous batch is being written
//! and acknowledged, so a worker holds at most two batches of buffers.
use crate::{
    chunks, remote_destination,
    scheduler::{FileOutcome, RelayJob, Work},
};
use futures::{FutureExt, TryFutureExt, future::try_join};
use nocterm_session::{
    FsError,
    fs::{RemoteDownload, RemoteUpload},
};
use nocterm_transfers::{
    StreamBudget, TransferCause, TransferEvent, TransferIssue, TransferOperation, TransferPath,
};

pub(crate) async fn run(work: Work, job: RelayJob) -> Result<FileOutcome, TransferIssue> {
    transfer(&work, &job)
        .await
        .map_err(|cause| issue(&job, TransferOperation::Write, cause))
}
async fn transfer(work: &Work, job: &RelayJob) -> Result<FileOutcome, TransferCause> {
    let Some((destination, mut reservation, number, mut writer)) =
        remote_destination::open_writer(work, job.remote()).await?
    else {
        return Ok(FileOutcome::Skipped(None));
    };
    let result = match receive(work, job, writer.as_mut()).await {
        Ok(()) => {
            remote_destination::publish(
                work,
                job.remote(),
                destination,
                number,
                &mut reservation,
                writer.as_mut(),
            )
            .await
        }
        Err(cause) => Err(cause),
    };
    if !matches!(result, Ok(FileOutcome::Completed))
        && let Err(error) = writer.cancel().await
    {
        work.batch().event(TransferEvent::Notice(issue(
            job,
            TransferOperation::Cleanup,
            error.into(),
        )));
    }
    result
}
async fn receive(
    work: &Work,
    job: &RelayJob,
    writer: &mut dyn RemoteUpload,
) -> Result<(), TransferCause> {
    let mut reader = work.remote(job.source_fs().download(job.source())).await?;
    let result = stream(work, job.expected(), reader.as_mut(), writer).await;
    let closed = reader.close().await.map_err(TransferCause::from);
    match result {
        Ok(()) => closed,
        Err(cause) => {
            if let Err(cleanup) = closed {
                work.batch().event(TransferEvent::Notice(issue(
                    job,
                    TransferOperation::Cleanup,
                    cleanup,
                )));
            }
            Err(cause)
        }
    }
}
async fn stream(
    work: &Work,
    expected: Option<u64>,
    reader: &mut dyn RemoteDownload,
    writer: &mut dyn RemoteUpload,
) -> Result<(), TransferCause> {
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
        let read = if eof {
            async { Ok::<_, FsError>(Vec::new()) }.boxed()
        } else {
            reader.read_batch(limits.chunk(), limits.acknowledgements())
        };
        let write = if writing.is_empty() {
            async { Ok::<_, FsError>(()) }.boxed()
        } else {
            writer.write_batch(writing)
        };
        let batch = try_join(read, write).map_ok(|(read, ())| read).boxed();
        let read = work.remote(batch).await?;
        if wrote > 0 {
            budget.acknowledged(wrote)?;
            work.batch().event(TransferEvent::Acknowledged {
                bytes: wrote as u64,
            });
        }
        if read.is_empty() {
            eof = true;
        } else {
            chunks::accept_batch(&budget, &read)?;
            pending = read;
        }
    }
    budget.finish()
}
fn issue(job: &RelayJob, operation: TransferOperation, cause: TransferCause) -> TransferIssue {
    TransferIssue::new(
        Some(TransferPath::Remote(job.source().to_owned())),
        operation,
        cause,
    )
}
