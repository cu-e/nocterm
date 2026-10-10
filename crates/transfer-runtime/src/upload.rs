use crate::{
    reservations,
    scheduler::{FileOutcome, UploadJob, Work},
};
use nocterm_session::{
    EntryKind, FsError,
    fs::{RemoteUpload, UploadMode, path},
    local_fs::{LocalFsError, LocalSource},
};
use nocterm_transfers::{
    CollisionPolicy, LimitKind, StreamBudget, TransferCause, TransferEvent, TransferIssue,
    TransferOperation, TransferPath,
};

struct SourceReader {
    source: Option<Box<dyn LocalSource>>,
}
impl SourceReader {
    fn new(source: Box<dyn LocalSource>) -> Self {
        Self {
            source: Some(source),
        }
    }
    fn take(&mut self) -> Result<Box<dyn LocalSource>, TransferCause> {
        self.source
            .take()
            .ok_or(TransferCause::Local(LocalFsError::Unsupported(
                "consumed source",
            )))
    }
    async fn metadata(
        &mut self,
        work: &Work,
    ) -> Result<nocterm_session::FileMetadata, TransferCause> {
        let source = self.take()?;
        let (source, result) = work
            .state()
            .blocking()
            .call(work.cancel(), move || {
                let result = source.metadata();
                Ok((source, result))
            })
            .await?;
        self.source = Some(source);
        result.map_err(Into::into)
    }
    async fn read(&mut self, work: &Work) -> Result<Vec<u8>, TransferCause> {
        let mut source = self.take()?;
        let size = work.state().limits().stream().chunk();
        let (source, result) = work
            .state()
            .blocking()
            .call(work.cancel(), move || {
                let result = source.read(size);
                Ok((source, result))
            })
            .await?;
        self.source = Some(source);
        result.map_err(Into::into)
    }
    async fn close(&mut self, work: &Work) -> Result<(), TransferCause> {
        if let Some(source) = self.source.take() {
            work.state()
                .blocking()
                .cleanup(move || source.close())
                .await?;
        }
        Ok(())
    }
}
pub(crate) async fn run(work: Work, job: UploadJob) -> Result<FileOutcome, TransferIssue> {
    let (source, remote, expected) = job.into_parts();
    let mut source = SourceReader::new(source);
    let outcome = transfer(&work, &remote, expected, &mut source).await;
    if let Err(cause) = source.close(&work).await {
        work.batch().event(TransferEvent::Notice(issue(
            &remote,
            TransferOperation::Cleanup,
            cause,
        )));
    }
    outcome.map_err(|cause| issue(&remote, TransferOperation::Write, cause))
}
async fn transfer(
    work: &Work,
    remote: &str,
    expected: Option<u64>,
    source: &mut SourceReader,
) -> Result<FileOutcome, TransferCause> {
    let metadata = source.metadata(work).await?;
    if metadata.is_symlink || metadata.kind != EntryKind::File {
        return Err(TransferCause::SpecialFile);
    }
    if expected.is_some_and(|size| metadata.size != Some(size)) {
        return Err(TransferCause::SizeChanged {
            expected,
            observed: metadata.size.unwrap_or(0),
        });
    }
    let Some((destination, mut reservation, number, mut writer)) =
        open_writer(work, remote).await?
    else {
        return Ok(FileOutcome::Skipped(None));
    };
    let result = stream(work, source, writer.as_mut(), metadata.size).await;
    let result = match result {
        Ok(()) => match source.close(work).await {
            Ok(()) => {
                publish(
                    work,
                    remote,
                    destination,
                    number,
                    &mut reservation,
                    writer.as_mut(),
                )
                .await
            }
            Err(cause) => Err(cause),
        },
        Err(cause) => Err(cause),
    };
    if !matches!(result, Ok(FileOutcome::Completed))
        && let Err(error) = writer.cancel().await
    {
        work.batch().event(TransferEvent::Notice(issue(
            remote,
            TransferOperation::Cleanup,
            error.into(),
        )));
    }
    result
}
async fn open_writer(
    work: &Work,
    original: &str,
) -> Result<
    Option<(
        String,
        reservations::Reservation,
        u32,
        Box<dyn RemoteUpload>,
    )>,
    TransferCause,
> {
    let mode = mode(work);
    let mut number = 0;
    loop {
        let Some((destination, reservation, used)) =
            reservations::remote(work, original, number).await?
        else {
            return Ok(None);
        };
        match work
            .remote(work.batch().request().fs().upload(&destination, mode))
            .await
        {
            Ok(writer) => return Ok(Some((destination, reservation, used, writer))),
            Err(TransferCause::Remote(FsError::AlreadyExists { .. })) => {
                drop(reservation);
                match work.batch().request().collisions() {
                    CollisionPolicy::Skip => return Ok(None),
                    CollisionPolicy::Rename => number = next(work, used)?,
                    CollisionPolicy::Replace => {
                        return Err(TransferCause::Remote(FsError::AlreadyExists {
                            path: destination,
                        }));
                    }
                }
            }
            Err(cause) => return Err(cause),
        }
    }
}
async fn stream(
    work: &Work,
    source: &mut SourceReader,
    writer: &mut dyn RemoteUpload,
    expected: Option<u64>,
) -> Result<(), TransferCause> {
    let limits = work.state().limits().stream();
    let mut budget = StreamBudget::new(limits, expected);
    loop {
        let chunks = read_batch(work, source, &budget).await?;
        if chunks.is_empty() {
            break;
        }
        let bytes = chunks.iter().map(Vec::len).sum::<usize>();
        work.remote(writer.write_batch(chunks)).await?;
        budget.acknowledged(bytes)?;
        work.batch().event(TransferEvent::Acknowledged {
            bytes: bytes as u64,
        });
    }
    budget.finish()?;
    let metadata = source.metadata(work).await?;
    if metadata.kind != EntryKind::File
        || metadata.is_symlink
        || metadata
            .size
            .is_some_and(|size| size != budget.transferred())
    {
        return Err(TransferCause::SizeChanged {
            expected,
            observed: metadata.size.unwrap_or(budget.transferred()),
        });
    }
    Ok(())
}
async fn read_batch(
    work: &Work,
    source: &mut SourceReader,
    budget: &StreamBudget,
) -> Result<Vec<Vec<u8>>, TransferCause> {
    let mut chunks = Vec::with_capacity(work.state().limits().stream().acknowledgements());
    let mut buffered = 0usize;
    for _ in 0..work.state().limits().stream().acknowledgements() {
        let bytes = source.read(work).await?;
        if bytes.is_empty() {
            break;
        }
        budget.accept_buffered(buffered, bytes.len())?;
        buffered += bytes.len();
        chunks.push(bytes);
    }
    Ok(chunks)
}
async fn publish(
    work: &Work,
    original: &str,
    mut destination: String,
    mut number: u32,
    reservation: &mut reservations::Reservation,
    writer: &mut dyn RemoteUpload,
) -> Result<FileOutcome, TransferCause> {
    loop {
        work.check()?;
        // Publication may commit while cancellation arrives. Await its answer
        // so a completed rename is never mistaken for an abandoned stage.
        match writer
            .publish_as(path::file_name(&destination).to_owned())
            .await
            .map_err(TransferCause::from)
        {
            Ok(_) => return Ok(FileOutcome::Completed),
            Err(TransferCause::Remote(FsError::AlreadyExists { .. }))
                if work.batch().request().collisions() == CollisionPolicy::Skip =>
            {
                return Ok(FileOutcome::Skipped(None));
            }
            Err(TransferCause::Remote(FsError::AlreadyExists { .. }))
                if work.batch().request().collisions() == CollisionPolicy::Rename =>
            {
                let Some((next_name, next_reservation, used)) =
                    reservations::remote(work, original, next(work, number)?).await?
                else {
                    return Ok(FileOutcome::Skipped(None));
                };
                *reservation = next_reservation;
                destination = next_name;
                number = used;
            }
            Err(cause) => return Err(cause),
        }
    }
}
fn next(work: &Work, number: u32) -> Result<u32, TransferCause> {
    let next = number
        .checked_add(1)
        .ok_or(TransferCause::Limit(LimitKind::RenameAttempts))?;
    if next > work.state().limits().discovery().rename_attempts() {
        Err(TransferCause::Limit(LimitKind::RenameAttempts))
    } else {
        Ok(next)
    }
}
fn mode(work: &Work) -> UploadMode {
    if work.batch().request().collisions() == CollisionPolicy::Replace {
        UploadMode::Replace
    } else {
        UploadMode::Create
    }
}
fn issue(remote: &str, operation: TransferOperation, cause: TransferCause) -> TransferIssue {
    TransferIssue::new(
        Some(TransferPath::Remote(remote.to_owned())),
        operation,
        cause,
    )
}
