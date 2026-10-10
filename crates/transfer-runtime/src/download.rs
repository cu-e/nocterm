use crate::{
    reservations,
    scheduler::{DownloadJob, FileOutcome, Work},
};
use nocterm_session::{
    fs::{RemoteDownload, UploadMode},
    local_fs::{LocalFsError, LocalStage, PublicationOutcome},
};
use nocterm_transfers::{
    CollisionPolicy, LimitKind, StreamBudget, TransferCause, TransferEvent, TransferIssue,
    TransferOperation, TransferPath,
};
use std::ffi::OsString;

struct StageWriter {
    stage: Option<Box<dyn LocalStage>>,
}
impl StageWriter {
    fn new(stage: Box<dyn LocalStage>) -> Self {
        Self { stage: Some(stage) }
    }
    fn take(&mut self) -> Result<Box<dyn LocalStage>, TransferCause> {
        self.stage
            .take()
            .ok_or(TransferCause::Local(LocalFsError::Unsupported(
                "consumed stage",
            )))
    }
    async fn write(&mut self, work: &Work, bytes: Vec<u8>) -> Result<(), TransferCause> {
        let mut stage = self.take()?;
        let (stage, result) = work
            .state()
            .blocking()
            .call(work.cancel(), move || {
                let result = stage.write(&bytes);
                Ok((stage, result))
            })
            .await?;
        self.stage = Some(stage);
        result.map_err(Into::into)
    }
    async fn sync(&mut self, work: &Work) -> Result<(), TransferCause> {
        let mut stage = self.take()?;
        let (stage, result) = work
            .state()
            .blocking()
            .call(work.cancel(), move || {
                let result = stage.sync();
                Ok((stage, result))
            })
            .await?;
        self.stage = Some(stage);
        result.map_err(Into::into)
    }
    async fn publish(
        &mut self,
        work: &Work,
        name: OsString,
    ) -> Result<PublicationOutcome, TransferCause> {
        let mut stage = self.take()?;
        let mode = if work.batch().request().collisions() == CollisionPolicy::Replace {
            UploadMode::Replace
        } else {
            UploadMode::Create
        };
        let (stage, result) = work
            .state()
            .blocking()
            .call(work.cancel(), move || {
                let result = stage.publish(&name, mode);
                Ok((stage, result))
            })
            .await?;
        self.stage = Some(stage);
        result.map_err(Into::into)
    }
    async fn cancel(&mut self, work: &Work) -> Result<(), TransferCause> {
        if let Some(stage) = self.stage.take() {
            work.state()
                .blocking()
                .cleanup(move || stage.cancel())
                .await?;
        }
        Ok(())
    }
}
pub(crate) async fn run(work: Work, job: DownloadJob) -> Result<FileOutcome, TransferIssue> {
    transfer(&work, &job)
        .await
        .map_err(|cause| issue(&job, TransferOperation::Write, cause))
}
async fn transfer(work: &Work, job: &DownloadJob) -> Result<FileOutcome, TransferCause> {
    let Some((name, reservation, number)) = reservations::local(work, job, 0).await? else {
        return Ok(FileOutcome::Skipped(None));
    };
    let directory = job.directory().clone();
    let stage = work
        .state()
        .blocking()
        .call(work.cancel(), move || directory.stage())
        .await?;
    let mut stage = StageWriter::new(stage);
    let result = receive(work, job, &mut stage).await;
    let result = match result {
        Ok(()) => publish(work, job, &mut stage, name, reservation, number).await,
        Err(cause) => Err(cause),
    };
    if !matches!(result, Ok(FileOutcome::Completed))
        && let Err(cause) = stage.cancel(work).await
    {
        work.batch().event(TransferEvent::Notice(issue(
            job,
            TransferOperation::Cleanup,
            cause,
        )));
    }
    result
}
async fn receive(
    work: &Work,
    job: &DownloadJob,
    stage: &mut StageWriter,
) -> Result<(), TransferCause> {
    let mut reader = work
        .remote(work.batch().request().fs().download(job.remote()))
        .await?;
    let result = stream(work, job, stage, reader.as_mut()).await;
    let closed = reader.close().await.map_err(TransferCause::from);
    match result {
        Ok(()) => {
            closed?;
            stage.sync(work).await
        }
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
    job: &DownloadJob,
    stage: &mut StageWriter,
    reader: &mut dyn RemoteDownload,
) -> Result<(), TransferCause> {
    let limits = work.state().limits().stream();
    let mut budget = StreamBudget::new(limits, job.expected());
    loop {
        let bytes = work.remote(reader.read(limits.chunk())).await?;
        if bytes.is_empty() {
            break;
        }
        budget.accept_chunk(bytes.len())?;
        let length = bytes.len();
        stage.write(work, bytes).await?;
        budget.acknowledged(length)?;
        work.batch().event(TransferEvent::Acknowledged {
            bytes: length as u64,
        });
    }
    budget.finish()
}
async fn publish(
    work: &Work,
    job: &DownloadJob,
    stage: &mut StageWriter,
    mut name: OsString,
    mut _reservation: reservations::Reservation,
    mut number: u32,
) -> Result<FileOutcome, TransferCause> {
    loop {
        match stage.publish(work, name.clone()).await {
            Ok(PublicationOutcome::Published) => return Ok(FileOutcome::Completed),
            Ok(PublicationOutcome::PublishedWithWarning(warning)) => {
                work.batch().event(TransferEvent::Notice(issue(
                    job,
                    TransferOperation::Publish,
                    TransferCause::Durability(warning),
                )));
                return Ok(FileOutcome::Completed);
            }
            Err(TransferCause::Local(LocalFsError::AlreadyExists(_)))
                if work.batch().request().collisions() == CollisionPolicy::Skip =>
            {
                return Ok(FileOutcome::Skipped(None));
            }
            Err(TransferCause::Local(LocalFsError::AlreadyExists(_)))
                if work.batch().request().collisions() == CollisionPolicy::Rename =>
            {
                let next = number
                    .checked_add(1)
                    .ok_or(TransferCause::Limit(LimitKind::RenameAttempts))?;
                if next > work.state().limits().discovery().rename_attempts() {
                    return Err(TransferCause::Limit(LimitKind::RenameAttempts));
                }
                let Some((next_name, next_reservation, used)) =
                    reservations::local(work, job, next).await?
                else {
                    return Ok(FileOutcome::Skipped(None));
                };
                name = next_name;
                _reservation = next_reservation;
                number = used;
            }
            Err(cause) => return Err(cause),
        }
    }
    // The named reservation is held until publication or failure exits this scope.
}
fn issue(job: &DownloadJob, operation: TransferOperation, cause: TransferCause) -> TransferIssue {
    TransferIssue::new(
        Some(TransferPath::Remote(job.remote().to_owned())),
        operation,
        cause,
    )
}
