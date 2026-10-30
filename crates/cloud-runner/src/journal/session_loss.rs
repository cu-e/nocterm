//! Exact native establishment verification and current-writer negative facts.
use super::{Driver, append, codec, head, metadata, replay, storage};
use crate::codec::archive::{MAX_ARCHIVE_BYTES, MAX_RECORDS, NativeRunArchive};
use nocterm_cloud_model::{authority::StreamBinding, identity::Fingerprint, journal::*};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

/// Stored snapshot/cipher/plaintext/archive generations plus bounded decoded
/// native record metadata. This is logical work, not SQLite/runtime RSS.
pub(super) fn work_bytes() -> usize {
    8 * (MAX_ARCHIVE_BYTES + 64 * 1024) + MAX_RECORDS * 16 * 1024
}

pub(super) fn finish(
    driver: &mut Driver,
    cleanup: &SessionLossCleanup,
) -> Result<(), JournalError> {
    driver.database.validate_leaves()?;
    let cipher = driver.cipher.as_ref();
    let stream = cleanup.lease().writer().stream();
    driver.catalog.require_stream(stream)?;
    let tx = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let mut catalog = driver.catalog.load(&tx, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&tx, &catalog)?;
    let previous = driver.heads.load(&tx, stream)?;
    let mut current = head::required(&tx, cipher, stream)?;
    current.require_cleanup(cleanup.lease())?;
    if current.position() != cleanup.expected() {
        return Err(JournalError::Conflict);
    }
    let maximum = driver
        .limits
        .chat_records
        .checked_add(1)
        .and_then(|records| records.checked_mul(driver.limits.chats))
        .ok_or(JournalError::Capacity)?;
    let original = SessionOriginal {
        db: &tx,
        cipher,
        stream,
        head: &current,
    };
    original.require(cleanup.original(), maximum, driver.limits.chats)?;
    let record = loss_record(cleanup)?;
    let batch = JournalBatch::new(
        cleanup.lease().writer().clone(),
        cleanup.expected(),
        None,
        vec![record],
        None,
        cleanup.outbox().to_vec(),
    )?;
    let next = batch.next_position()?;
    let charge = current.budget().appended(&batch)?;
    if next.control() != current.position().control()
        || charge.total() > driver.limits.chat_bytes
        || next
            .records()
            .get()
            .saturating_sub(current.retained().get())
            > driver.limits.chat_records
    {
        return Err(JournalError::Capacity);
    }
    append::records(&tx, cipher, &batch)?;
    append::outbox(&tx, cipher, &batch)?;
    current.update(next, current.deleted(), charge);
    current.save(&tx, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish(
            &tx,
            &mut catalog,
            &current,
            previous.as_ref(),
            batch.outbox().last().map(ResidentPublication::entry),
        )?;
    tx.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)?;
    Ok(())
}
fn loss_record(cleanup: &SessionLossCleanup) -> Result<JournalRecord, JournalError> {
    let record = JournalRecord::new(
        RecordContext::Run(cleanup.observation().binding().run().clone()),
        JournalRecordKind::RunStatus,
        cleanup.payload().clone(),
    )?;
    let (writer, actual) = crate::coordinator::session_fact::decode_record(&record)?;
    if writer != cleanup.lease().writer().epoch() || &actual != cleanup.observation() {
        return Err(JournalError::Conflict);
    }
    Ok(record)
}

struct SessionOriginal<'a> {
    db: &'a Connection,
    cipher: &'a dyn JournalCipher,
    stream: &'a StreamBinding,
    head: &'a head::Head,
}
impl SessionOriginal<'_> {
    fn require(
        &self,
        anchor: &SessionEstablishedAnchor,
        maximum: u64,
        chats: u64,
    ) -> Result<(), JournalError> {
        entries_preflight(self.db, maximum)?;
        let stored = if anchor.sequence() > self.head.retained() {
            let before = nocterm_cloud_model::identity::JournalSequence::from_u64(
                anchor
                    .sequence()
                    .get()
                    .checked_sub(1)
                    .ok_or(JournalError::Corrupt)?,
            );
            let page = replay::read_from(
                self.db,
                self.cipher,
                self.stream,
                self.head,
                before,
                JournalReadLimits::new(1, MAX_SESSION_FACT_BYTES + 16 * 1024)?,
            )?;
            page.records()
                .first()
                .cloned()
                .ok_or(JournalError::Conflict)?
        } else {
            self.archived(anchor, chats)?
        };
        if stored.stream() != self.stream
            || stored.sequence() != anchor.sequence()
            || stored.revision() != anchor.revision()
            || stored.record().context().run() != Some(anchor.observation().binding().run())
            || Fingerprint::from_bytes(Sha256::digest(stored.record().payload().as_bytes()).into())
                != anchor.payload_digest()
        {
            return Err(JournalError::Conflict);
        }
        let (writer, observation) =
            crate::coordinator::session_fact::decode_record(stored.record())?;
        if writer != anchor.writer() || &observation != anchor.observation() {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }
    fn archived(
        &self,
        anchor: &SessionEstablishedAnchor,
        chats: u64,
    ) -> Result<StoredRecord, JournalError> {
        metadata::snapshot_rows(self.db, chats)?;
        let size = self
            .db
            .query_row(
                "SELECT length(body) FROM snapshots WHERE chat=?1",
                [head::key(self.stream).as_slice()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(storage)?
            .ok_or(JournalError::Conflict)?;
        if head::check_length(size)? > MAX_ARCHIVE_BYTES + 64 * 1024 {
            return Err(JournalError::Capacity);
        }
        let (context, body) = replay::object(
            self.db,
            self.stream,
            JournalStorageClass::Snapshot,
            self.head.position(),
        )?
        .ok_or(JournalError::Conflict)?;
        let snapshot = codec::open::<codec::Snapshot>(self.cipher, &context, &body)?.restore()?;
        if snapshot.stream() != self.stream
            || snapshot.at().revision() != context.revision()
            || snapshot.at().records() != context.sequence()
            || snapshot.at().records() < anchor.sequence()
        {
            return Err(JournalError::Corrupt);
        }
        NativeRunArchive::decode(&snapshot)?
            .into_records()
            .into_iter()
            .find(|record| record.sequence() == anchor.sequence())
            .ok_or(JournalError::Conflict)
    }
}
fn entries_preflight(db: &Connection, maximum: u64) -> Result<(), JournalError> {
    metadata::require_transaction(db)?;
    metadata::scan(
        db,
        "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(sequence)!='integer' OR typeof(revision)!='integer' OR typeof(writer)!='integer' OR typeof(charge)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM entries LIMIT ?1",
        maximum,
        0,
    )?;
    metadata::scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR sequence<=0 OR revision<=0 OR writer<=0 OR charge<=0 OR length(body)<=0 OR length(body)>?2 THEN 0 ELSE 1 END FROM entries LIMIT ?1",
        maximum,
        MAX_STORED_JOURNAL_BYTES,
    )?;
    Ok(())
}
