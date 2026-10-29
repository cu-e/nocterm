use super::{Effect, load, save};
use crate::journal::{Driver, append, head, storage};
use nocterm_cloud_model::{
    journal::*,
    payload::OpaqueBytes,
    tool::{BackgroundCommand, BackgroundCommandOutcome, ToolExecutionError},
};
use nocterm_core::SharedCause;
use rusqlite::TransactionBehavior;

fn observe(
    driver: &mut Driver,
    command: &BackgroundCommand,
    outcome: BackgroundRecordOutcome,
    cleanup: Option<&CleanupLease>,
) -> Result<(), JournalError> {
    let events = driver.events.as_ref().ok_or(JournalError::InvalidBatch)?;
    driver.database.validate_leaves()?;
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let intent = command.dispatch().intent();
    let stream = intent.authority().run().stream();
    driver.catalog.require_stream(&stream)?;
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    let previous = driver.heads.load(&db, &stream)?;
    let mut current = head::required(&db, driver.cipher.as_ref(), &stream)?;
    let lease = if let Some(cleanup) = cleanup {
        if outcome == BackgroundRecordOutcome::Retained {
            return Err(JournalError::InvalidBatch);
        }
        current.require_cleanup(cleanup)?;
        cleanup.writer().clone()
    } else {
        let lease = WriterLease::new(stream.clone(), command.dispatch().commit().writer());
        current.require_writer(&lease)?;
        lease
    };
    let mut effect: Effect =
        load(&db, driver.cipher.as_ref(), &stream, intent.id())?.ok_or(JournalError::Conflict)?;
    if !effect.background(command, outcome)? {
        return Ok(());
    }
    let record = JournalRecord::new(
        RecordContext::Run(intent.authority().run().clone()),
        JournalRecordKind::BackgroundCommand {
            intent: intent.id(),
            terminal: command.terminal().to_owned(),
            command: command.command().to_owned(),
            outcome,
        },
        OpaqueBytes::new(Vec::<u8>::new())?,
    )?;
    let next = current.position().next(1, 1, false)?;
    let work = if outcome == BackgroundRecordOutcome::Retained {
        events.1.ordinary()
    } else {
        events.1.cleanup()
    };
    let event = events
        .0
        .encode(&lease, next, command, outcome, work.as_ref())?;
    let batch = JournalBatch::new(
        lease,
        current.position(),
        None,
        vec![record],
        None,
        vec![event],
    )?;
    let charge = current.budget().appended(&batch)?;
    if charge.total() > driver.limits.chat_bytes
        || next
            .records()
            .get()
            .saturating_sub(current.retained().get())
            > driver.limits.chat_records
    {
        return Err(JournalError::Capacity);
    }
    effect.validate()?;
    save(
        &db,
        driver.cipher.as_ref(),
        &stream,
        intent.id(),
        next.records(),
        next.revision(),
        batch.lease().epoch(),
        &effect,
    )?;
    append::records(&db, driver.cipher.as_ref(), &batch)?;
    append::outbox(&db, driver.cipher.as_ref(), &batch)?;
    current.set_active(super::active_after(
        &db,
        driver.cipher.as_ref(),
        &current,
        &batch,
    )?);
    current.update(next, current.deleted(), charge);
    current.save(&db, driver.cipher.as_ref())?;
    driver
        .heads
        .publication(
            driver.cipher.as_ref(),
            &driver.catalog,
            driver.catalog_cipher.as_ref(),
        )
        .publish(
            &db,
            &mut catalog,
            &current,
            previous.as_ref(),
            batch.outbox().last().map(ResidentPublication::entry),
        )?;
    db.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)?;
    Ok(())
}

pub(super) fn retain(
    driver: &mut Driver,
    command: &BackgroundCommand,
) -> Result<(), ToolExecutionError> {
    observe(driver, command, BackgroundRecordOutcome::Retained, None).map_err(tool_error)
}
pub(super) fn finish(
    driver: &mut Driver,
    command: &BackgroundCommand,
    outcome: BackgroundCommandOutcome,
) -> Result<(), ToolExecutionError> {
    observe(
        driver,
        command,
        match outcome {
            BackgroundCommandOutcome::ObservedExit(exit) => {
                BackgroundRecordOutcome::ObservedExit(exit)
            }
            BackgroundCommandOutcome::Unknown => BackgroundRecordOutcome::Unknown,
        },
        None,
    )
    .map_err(tool_error)
}
fn tool_error(error: JournalError) -> ToolExecutionError {
    match error {
        JournalError::Capacity => ToolExecutionError::Capacity,
        JournalError::Fenced => ToolExecutionError::Revoked,
        JournalError::InvalidBatch => ToolExecutionError::UnsupportedCapability,
        error => ToolExecutionError::Execution(SharedCause::new(error)),
    }
}

pub(in crate::journal) fn cleanup(
    driver: &mut Driver,
    lease: &CleanupLease,
    command: &BackgroundCommand,
    outcome: BackgroundCommandOutcome,
) -> Result<(), JournalError> {
    observe(
        driver,
        command,
        match outcome {
            BackgroundCommandOutcome::ObservedExit(exit) => {
                BackgroundRecordOutcome::ObservedExit(exit)
            }
            BackgroundCommandOutcome::Unknown => BackgroundRecordOutcome::Unknown,
        },
        Some(lease),
    )
}
