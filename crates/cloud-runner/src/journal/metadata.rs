//! Two-phase metadata checks before SQLite can materialize keys, bodies or sorts.
use super::{integer, storage};
use nocterm_cloud_model::journal::{JournalError, MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT};
use rusqlite::{Connection, params};

// Phase one uses ONLY typeof(Column), whose bundled SQLite OP_Column carries
// TYPEOFARG and omits payload content. In particular, do not call length(TEXT).
const CATALOG_TYPES: &str = "SELECT CASE WHEN typeof(singleton)!='integer' OR typeof(revision)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM account_catalog LIMIT ?1";
const CHAT_TYPES: &str = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(binding)!='blob' OR typeof(scope)!='blob' OR typeof(chat_id)!='blob' OR typeof(runner)!='blob' OR typeof(body)!='blob' OR typeof(header_digest)!='blob' OR typeof(key_epoch)!='integer' OR typeof(writer)!='integer' OR typeof(revision)!='integer' OR typeof(records)!='integer' OR typeof(events)!='integer' OR typeof(control)!='integer' OR typeof(retained)!='integer' OR typeof(deleted)!='integer' OR typeof(charge)!='integer' THEN 0 ELSE 1 END FROM chats LIMIT ?1";
const HEAD_TYPES: &str = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(digest)!='blob' OR typeof(body)!='blob' OR typeof(revision)!='integer' OR typeof(writer) NOT IN ('null','integer') THEN 0 ELSE 1 END FROM current_heads LIMIT ?1";

// Phase two runs only after the complete bounded type pass succeeded in the
// SAME transaction. BLOB length(Column) carries LENGTHARG; all ordinary scalar
// column loads below have already been proved to be fixed-size integers/null.
const CATALOG_BOUNDS: &str = "SELECT CASE WHEN singleton!=1 OR revision<=0 OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM account_catalog LIMIT ?1";
const CHAT_BOUNDS: &str = "SELECT CASE WHEN length(chat)!=32 OR length(binding)!=32 OR length(scope)!=32 OR length(chat_id)!=16 OR length(runner)!=16 OR length(header_digest)!=32 OR length(body)<=0 OR length(body)>?2 OR key_epoch<=0 OR writer<=0 OR revision<0 OR records<0 OR events<0 OR control<0 OR retained<0 OR deleted NOT IN (0,1) OR charge<0 THEN 0 ELSE 1 END FROM chats LIMIT ?1";
const HEAD_BOUNDS: &str = "SELECT CASE WHEN length(chat)!=32 OR length(digest)!=32 OR length(body)<=0 OR length(body)>?2 OR revision<=0 OR (writer IS NOT NULL AND writer<=0) THEN 0 ELSE 1 END FROM current_heads LIMIT ?1";

pub(super) fn catalog(db: &Connection, maximum: usize) -> Result<(), JournalError> {
    require_transaction(db)?;
    let count = scan(db, CATALOG_TYPES, 1, 0)?;
    if count != 1 {
        return Err(JournalError::Corrupt);
    }
    scan(db, CATALOG_BOUNDS, 1, maximum)?;
    Ok(())
}
pub(super) fn rows(db: &Connection, maximum: u64) -> Result<(), JournalError> {
    require_transaction(db)?;
    let chats = scan(db, CHAT_TYPES, maximum, 0)?;
    let heads = scan(db, HEAD_TYPES, maximum, 0)?;
    if chats != heads {
        return Err(JournalError::Corrupt);
    }
    scan(
        db,
        CHAT_BOUNDS,
        maximum,
        super::header_link::MAX_HEADER_CIPHERTEXT,
    )?;
    scan(db, HEAD_BOUNDS, maximum, MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT)?;
    Ok(())
}
pub(super) fn outbox_rows(db: &Connection, maximum: u64) -> Result<(), JournalError> {
    require_transaction(db)?;
    scan(
        db,
        "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(sequence)!='integer' OR typeof(id)!='blob' OR typeof(delivery)!='blob' OR typeof(record)!='integer' OR typeof(revision)!='integer' OR typeof(writer)!='integer' OR typeof(body)!='blob' OR typeof(acknowledged)!='integer' THEN 0 ELSE 1 END FROM outbox LIMIT ?1",
        maximum,
        0,
    )?;
    scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR length(id)!=16 OR length(delivery)!=16 OR sequence<=0 OR record<=0 OR revision<=0 OR writer<=0 OR acknowledged NOT IN (0,1) OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM outbox LIMIT ?1",
        maximum,
        super::MAX_STORED_JOURNAL_BYTES,
    )?;
    Ok(())
}
pub(super) fn snapshot_rows(db: &Connection, chats: u64) -> Result<(), JournalError> {
    require_transaction(db)?;
    let transfer_types = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(revision)!='integer' OR typeof(sequence)!='integer' OR typeof(writer)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM snapshot_transfers LIMIT ?1";
    let payload_types = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(kind)!='integer' OR typeof(revision)!='integer' OR typeof(sequence)!='integer' OR typeof(writer)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM snapshot_payloads LIMIT ?1";
    let archive_types = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(revision)!='integer' OR typeof(sequence)!='integer' OR typeof(writer)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM snapshots LIMIT ?1";
    scan(db, transfer_types, chats, 0)?;
    scan(db, archive_types, chats, 0)?;
    scan(
        db,
        payload_types,
        chats.checked_mul(3).ok_or(JournalError::Capacity)?,
        0,
    )?;
    scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR revision<=0 OR sequence<=0 OR writer<=0 OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM snapshot_transfers LIMIT ?1",
        chats,
        16 * 1024,
    )?;
    scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR kind NOT IN (1,2,3) OR revision<=0 OR sequence<=0 OR writer<=0 OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM snapshot_payloads LIMIT ?1",
        chats.checked_mul(3).ok_or(JournalError::Capacity)?,
        super::MAX_STORED_JOURNAL_BYTES,
    )?;
    scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR revision<=0 OR sequence<=0 OR writer<=0 OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM snapshots LIMIT ?1",
        chats,
        super::MAX_STORED_JOURNAL_BYTES,
    )?;
    Ok(())
}
pub(super) fn require_transaction(db: &Connection) -> Result<(), JournalError> {
    if db.is_autocommit() {
        Err(JournalError::InvalidBatch)
    } else {
        Ok(())
    }
}
pub(super) fn scan(
    db: &Connection,
    sql: &str,
    maximum: u64,
    bytes: usize,
) -> Result<u64, JournalError> {
    let mut statement = db.prepare(sql).map_err(storage)?;
    let end = maximum.checked_add(1).ok_or(JournalError::Capacity)?;
    let mut rows = if statement.parameter_count() == 1 {
        statement.query([integer(end)?]).map_err(storage)?
    } else {
        statement
            .query(params![integer(end)?, integer(bytes as u64)?])
            .map_err(storage)?
    };
    let mut count = 0u64;
    while let Some(row) = rows.next().map_err(storage)? {
        count += 1;
        if count > maximum {
            return Err(JournalError::Capacity);
        }
        if row.get::<_, i64>(0).map_err(storage)? != 1 {
            return Err(JournalError::Corrupt);
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_vm_reads_types_and_blob_lengths_without_copying_raw_values_or_sorting() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        for sql in [CATALOG_TYPES, CHAT_TYPES, HEAD_TYPES] {
            let mut statement = db.prepare(&format!("EXPLAIN {sql}")).unwrap();
            let rows = statement
                .query_map([2i64], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(6)?))
                })
                .unwrap();
            let mut columns = 0;
            for row in rows {
                let (opcode, flags) = row.unwrap();
                assert!(!matches!(
                    opcode.as_str(),
                    "Sort" | "SorterOpen" | "SorterInsert" | "OpenAutoindex"
                ));
                if opcode == "Column" {
                    columns += 1;
                    assert_ne!(flags & 0x80, 0, "unbounded type preflight: {sql}");
                }
            }
            assert!(columns > 0);
        }
        for (sql, blob_columns) in [
            (CATALOG_BOUNDS, &[2i64][..]),
            (CHAT_BOUNDS, &[0, 1, 2, 3, 4, 14, 15][..]),
            (HEAD_BOUNDS, &[0, 3, 4][..]),
        ] {
            let mut statement = db.prepare(&format!("EXPLAIN {sql}")).unwrap();
            let rows = statement
                .query_map(params![2i64, 8192i64], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(6)?,
                    ))
                })
                .unwrap();
            for row in rows {
                let (opcode, column, flags) = row.unwrap();
                assert!(!matches!(
                    opcode.as_str(),
                    "Sort" | "SorterOpen" | "SorterInsert" | "OpenAutoindex"
                ));
                if opcode == "Column" && blob_columns.contains(&column) {
                    assert_ne!(flags & 0x40, 0, "raw blob in bounds preflight: {sql}");
                }
            }
        }
    }
}
