use super::{Driver, head, storage};
use nocterm_cloud_model::{identity::*, journal::*};
use rusqlite::TransactionBehavior;

pub(super) fn acknowledge(
    driver: &mut Driver,
    lease: &WriterLease,
    through: EventSequence,
) -> Result<(), JournalError> {
    driver.database.validate_leaves()?;
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    let previous = driver.heads.load(&db, lease.stream())?;
    let mut head = head::required(&db, driver.cipher.as_ref(), lease.stream())?;
    // A deleted chat may still deliver its final tombstone outbox.
    if head.lease() != lease {
        return Err(JournalError::Fenced);
    }
    if through > head.position().events() {
        return Err(JournalError::Conflict);
    }
    if through <= head.acknowledged() {
        return Ok(());
    }
    head.acknowledge(through);
    head.save(&db, driver.cipher.as_ref())?;
    driver
        .heads
        .publication(
            driver.cipher.as_ref(),
            &driver.catalog,
            driver.catalog_cipher.as_ref(),
        )
        .publish(&db, &mut catalog, &head, previous.as_ref(), None)?;
    db.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)
}

pub(super) fn compact(
    driver: &mut Driver,
    lease: &WriterLease,
    expected: JournalPosition,
    snapshot: &JournalSnapshot,
) -> Result<(), JournalError> {
    let db = driver.connection.transaction().map_err(storage)?;
    let catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    super::metadata::snapshot_rows(&db, driver.limits.chats)?;
    let head = head::required(&db, driver.cipher.as_ref(), lease.stream())?;
    head.require_cleanup(&CleanupLease::for_writer(lease.clone()))?;
    let state = super::snapshot_publication::storage::load(&db, driver.cipher.as_ref(), &head)?;
    let active = state.active.ok_or(JournalError::Conflict)?.restore()?;
    use sha2::{Digest, Sha256};
    if active.covered() != expected
        || snapshot.at() != expected
        || snapshot.stream() != lease.stream()
        || head.retained() != expected.records()
        || active.archive_length() != snapshot.payload().len()
        || active.archive().as_bytes().as_slice()
            != Sha256::digest(snapshot.payload().as_bytes()).as_slice()
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}
