//! Explicit schema upgrades of an existing journal database.
//!
//! Only the immediately previous format is upgraded; every other version stays
//! `UnsupportedFormat` and needs explicit recovery.
//!
//! Version 5 to 6 adds `outbox.retry_until`, the promised retry horizon of a publication
//! (milliseconds since the epoch, `0` for none). Entries stored by the new code get
//! `now + OUTBOX_RETRY_WINDOW_MS` once, when they are written. Entries that exist at the
//! upgrade and are not yet acknowledged get the same horizon computed once from the time
//! of the upgrade; acknowledged entries get `0` because nothing will be published again.
use crate::storage::storage;
use nocterm_cloud_model::journal::{JournalError, OUTBOX_RETRY_WINDOW_MS};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

/// The only format `upgrade` accepts.
pub(super) const PREVIOUS_VERSION: i64 = 5;

pub(super) fn now_ms() -> Result<i64, JournalError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or(JournalError::Corrupt)
}

/// The horizon for an entry stored at `now_ms`.
pub(super) fn horizon_from(now_ms: i64) -> i64 {
    now_ms.saturating_add(OUTBOX_RETRY_WINDOW_MS)
}

/// Upgrades a version 5 database to the current one in one transaction.
pub(super) fn upgrade(db: &Connection) -> Result<(), JournalError> {
    upgrade_at(db, now_ms()?)
}

fn upgrade_at(db: &Connection, now_ms: i64) -> Result<(), JournalError> {
    let tx = db.unchecked_transaction().map_err(storage)?;
    tx.execute_batch(
        "ALTER TABLE outbox ADD COLUMN retry_until INTEGER NOT NULL DEFAULT 0 CHECK(retry_until>=0)",
    )
    .map_err(storage)?;
    tx.execute(
        "UPDATE outbox SET retry_until=?1 WHERE acknowledged=0",
        [horizon_from(now_ms)],
    )
    .map_err(storage)?;
    tx.pragma_update(None, "user_version", super::SCHEMA_VERSION)
        .map_err(storage)?;
    tx.commit().map_err(storage)
}
