//! Actual stored receipt deduplication and FIFO watermark, distinct from C.
use super::*;
use crate::codec as native;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Receipt {
    delivery: [u8; 16],
    digest: [u8; 32],
    cursor: u64,
}

pub(super) fn record(
    driver: &mut Driver,
    lease: &CleanupLease,
    receipt: &OutboxStored,
    work: &dyn PublicationWorkBudget,
) -> Result<EventSequence, JournalError> {
    if receipt.stream() != lease.writer().stream() {
        return Err(JournalError::Conflict);
    }
    let cipher = driver.cipher.as_ref();
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    super::super::metadata::snapshot_rows(&db, driver.limits.chats)?;
    let previous = driver.heads.required(&db, lease.writer().stream())?;
    let mut head = head::required(&db, cipher, lease.writer().stream())?;
    head.require_cleanup(lease)?;
    preflight(
        &db,
        driver
            .limits
            .chat_records
            // One pending maintenance marker per chat, before any prefix purge.
            // Ordinary admission still counts against chat_records unchanged.
            .checked_add(1)
            .and_then(|records| records.checked_mul(driver.limits.chats))
            .ok_or(JournalError::Capacity)?,
    )?;
    let delivery = StoredDelivery::load(&db, cipher, &head, receipt, work)?;
    if delivery.existing(&db, cipher)? {
        return Ok(head.acknowledged());
    }
    if delivery.resident.sequence() != head.acknowledged().advance(1)? {
        return Err(JournalError::Conflict);
    }
    delivery.persist(&db, cipher, &mut head)?;
    head.save(&db, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish(&db, &mut catalog, &head, Some(&previous), None)?;
    db.commit().map_err(sql_error)?;
    driver.catalog.observe(&catalog)?;
    Ok(delivery.resident.sequence())
}
struct StoredDelivery {
    resident: ResidentPublication,
    context: JournalCipherContext,
    receipt: OutboxStored,
}
impl StoredDelivery {
    fn load(
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &head::Head,
        receipt: &OutboxStored,
        work: &dyn PublicationWorkBudget,
    ) -> Result<Self, JournalError> {
        let sequence: i64 = db
            .query_row(
                "SELECT sequence FROM outbox WHERE chat=?1 AND delivery=?2",
                params![
                    head::key(receipt.stream()).as_slice(),
                    receipt.delivery_id().as_bytes().as_slice()
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?
            .ok_or(JournalError::NotFound)?;
        let sequence = EventSequence::from_u64(super::super::unsigned(sequence)?);
        let after =
            EventSequence::from_u64(sequence.get().checked_sub(1).ok_or(JournalError::Corrupt)?);
        let allocation = storage::reserve_decode(
            work,
            storage::outbox_length(db, receipt.stream(), sequence)?,
        )?;
        let entry = super::super::replay::outbox_from(
            db,
            cipher,
            receipt.stream(),
            head,
            after,
            JournalReadLimits::new(1, MAX_TRANSACTION_BYTES)?,
        )?
        .pop()
        .ok_or(JournalError::Corrupt)?;
        if entry.delivery_id() != receipt.delivery_id()
            || entry.anchor().ciphertext() != *receipt.ciphertext()
            || Sha256::digest(entry.sealed().as_bytes()).as_slice()
                != receipt.ciphertext().as_bytes()
        {
            return Err(JournalError::Conflict);
        }

        let context = JournalCipherContext::new(
            receipt.stream().clone(),
            JournalStorageClass::Outbox,
            *receipt.delivery_id().as_bytes(),
            entry.record(),
            entry.anchor().position().revision(),
            entry.anchor().position().writer(),
        );

        let resident = ResidentPublication::new(receipt.stream().clone(), entry, allocation)?;
        Ok(Self {
            resident,
            context,
            receipt: receipt.clone(),
        })
    }
    fn existing(&self, db: &Connection, cipher: &dyn JournalCipher) -> Result<bool, JournalError> {
        let old: Option<Vec<u8>> = db
            .query_row(
                "SELECT body FROM outbox_deliveries WHERE chat=?1 AND sequence=?2",
                params![
                    head::key(self.receipt.stream()).as_slice(),
                    integer(self.resident.sequence().get())?
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some(old) = old {
            let old: Receipt = native::open(cipher, &self.context, &old)?;
            if old.delivery != *self.receipt.delivery_id().as_bytes()
                || old.digest != *self.receipt.ciphertext().as_bytes()
                || old.cursor != self.receipt.cursor().get()
            {
                return Err(JournalError::Conflict);
            }
            return Ok(true);
        }

        Ok(false)
    }
    fn persist(
        &self,
        db: &Connection,
        cipher: &dyn JournalCipher,
        head: &mut head::Head,
    ) -> Result<(), JournalError> {
        let body = native::seal(
            cipher,
            &self.context,
            &Receipt {
                delivery: *self.receipt.delivery_id().as_bytes(),
                digest: *self.receipt.ciphertext().as_bytes(),
                cursor: self.receipt.cursor().get(),
            },
        )?;
        db.execute(
            "INSERT INTO outbox_deliveries(chat,sequence,body) VALUES(?1,?2,?3)",
            params![
                head::key(self.receipt.stream()).as_slice(),
                integer(self.resident.sequence().get())?,
                body
            ],
        )
        .map_err(sql_error)?;
        // Snapshot ACK is atomic with its durable retention descriptor.
        if self.resident.anchor().kind() == HeadAnchorKind::Snapshot {
            let mut state = storage::load(db, cipher, head)?;
            let candidate = state
                .pending
                .take()
                .ok_or(JournalError::NotFound)?
                .restore()?
                .record_stored(self.receipt.clone())?;
            state.pending = Some((&candidate).into());
            storage::save(db, cipher, head, &state)?;
        }
        head.acknowledge(self.resident.sequence());

        Ok(())
    }
}
fn preflight(db: &Connection, maximum: u64) -> Result<(), JournalError> {
    super::super::metadata::outbox_rows(db, maximum)?;
    let sql = "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(sequence)!='integer' OR typeof(delivery)!='blob' THEN 0 ELSE 1 END FROM outbox LIMIT ?1";
    super::super::metadata::scan(db, sql, maximum, 0)?;
    super::super::metadata::scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR length(delivery)!=16 OR sequence<=0 THEN 0 ELSE 1 END FROM outbox LIMIT ?1",
        maximum,
        0,
    )?;
    super::super::metadata::scan(
        db,
        "SELECT CASE WHEN typeof(chat)!='blob' OR typeof(sequence)!='integer' OR typeof(body)!='blob' THEN 0 ELSE 1 END FROM outbox_deliveries LIMIT ?1",
        maximum,
        0,
    )?;
    super::super::metadata::scan(
        db,
        "SELECT CASE WHEN length(chat)!=32 OR sequence<=0 OR length(body)<=0 OR length(body)>4096 THEN 0 ELSE 1 END FROM outbox_deliveries LIMIT ?1",
        maximum,
        0,
    )?;
    Ok(())
}
