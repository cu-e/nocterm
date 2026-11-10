//! Streams one file from a source filesystem into a staged destination upload.
//!
//! The next read batch is in flight while the previous batch is being written
//! and acknowledged, so a worker holds at most two batches of buffers.
use crate::{
    pipeline, remote_destination,
    scheduler::{FileOutcome, RelayJob, Work},
};
use nocterm_session::fs::RemoteUpload;
use nocterm_transfers::{
    TransferCause, TransferEvent, TransferIssue, TransferOperation, TransferPath,
};

/// A failure and the side of the relay that raised it: the source host is read, the
/// destination host written, and the agent needs to know which one to look at.
type Sided = (TransferOperation, TransferCause);
fn source_failed(cause: TransferCause) -> Sided {
    (TransferOperation::Read, cause)
}
fn destination_failed(cause: TransferCause) -> Sided {
    (TransferOperation::Write, cause)
}

pub(crate) async fn run(work: Work, job: RelayJob) -> Result<FileOutcome, TransferIssue> {
    transfer(&work, &job)
        .await
        .map_err(|(operation, cause)| issue(&job, operation, cause))
}
async fn transfer(work: &Work, job: &RelayJob) -> Result<FileOutcome, Sided> {
    let Some((destination, mut reservation, number, mut writer)) =
        remote_destination::open_writer(work, job.remote())
            .await
            .map_err(destination_failed)?
    else {
        return Ok(FileOutcome::Skipped(None));
    };
    let result = match receive(work, job, writer.as_mut()).await {
        Ok(()) => remote_destination::publish(
            work,
            job.remote(),
            destination,
            number,
            &mut reservation,
            writer.as_mut(),
        )
        .await
        .map_err(destination_failed),
        Err(failure) => Err(failure),
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
async fn receive(work: &Work, job: &RelayJob, writer: &mut dyn RemoteUpload) -> Result<(), Sided> {
    let mut reader = work
        .remote(job.source_fs().download(job.source()))
        .await
        .map_err(source_failed)?;
    let result = pipeline::copy(work, job.expected(), reader.as_mut(), writer).await;
    let closed = reader.close().await.map_err(TransferCause::from);
    match result {
        Ok(()) => closed.map_err(source_failed),
        Err(failure) => {
            if let Err(cleanup) = closed {
                work.batch().event(TransferEvent::Notice(issue(
                    job,
                    TransferOperation::Cleanup,
                    cleanup,
                )));
            }
            Err(failure)
        }
    }
}
fn issue(job: &RelayJob, operation: TransferOperation, cause: TransferCause) -> TransferIssue {
    TransferIssue::new(
        Some(TransferPath::Remote(job.source().to_owned())),
        operation,
        cause,
    )
}
