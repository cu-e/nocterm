//! Explicit schema upgrades of an existing journal database.
//!
//! Versions 5 and 6 are upgraded in order, each step in its own transaction; every other
//! version stays `UnsupportedFormat` and needs explicit recovery.
//!
//! Version 5 to 6 adds `outbox.retry_until`, the promised retry horizon of a publication
//! (milliseconds since the epoch, `0` for none). Entries stored by the new code get
//! `now + OUTBOX_RETRY_WINDOW_MS` once, when they are written. Entries that exist at the
//! upgrade and are not yet acknowledged get the same horizon computed once from the time
//! of the upgrade; acknowledged entries get `0` because nothing will be published again.
//!
//! Version 6 to 7 adds the empty `chat_summaries` table (ADR 0135). Existing chats get no
//! row here: deriving a summary needs the chat keys and the coordinator's record codec, so
//! the runner backfills each chat from its journal at its first summary sweep.
use crate::storage::storage;
use nocterm_cloud_model::journal::{JournalError, OUTBOX_RETRY_WINDOW_MS};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

/// The oldest format `upgrade` accepts.
const OLDEST_VERSION: i64 = 5;

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

/// Whether `upgrade` accepts a database at `version`.
pub(super) fn upgradable(version: i64) -> bool {
    (OLDEST_VERSION..super::SCHEMA_VERSION).contains(&version)
}

/// Upgrades a database from `version` to the current one, one version per transaction.
pub(super) fn upgrade(db: &Connection, version: i64) -> Result<(), JournalError> {
    if !upgradable(version) {
        return Err(JournalError::UnsupportedFormat);
    }
    if version == 5 {
        to_v6(db, now_ms()?)?;
    }
    to_v7(db)
}

fn to_v6(db: &Connection, now_ms: i64) -> Result<(), JournalError> {
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
    tx.pragma_update(None, "user_version", 6).map_err(storage)?;
    tx.commit().map_err(storage)
}

fn to_v7(db: &Connection) -> Result<(), JournalError> {
    let tx = db.unchecked_transaction().map_err(storage)?;
    tx.execute_batch(super::summaries::TABLE).map_err(storage)?;
    tx.pragma_update(None, "user_version", 7).map_err(storage)?;
    tx.commit().map_err(storage)
}
