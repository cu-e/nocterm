use super::{Driver, codec, effects, head, integer, storage, unsigned};
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(super) fn commit(
    driver: &mut Driver,
    batch: &JournalBatch,
) -> Result<CommitOutcome, JournalError> {
    if batch.records().iter().any(|record| {
        matches!(
            record.kind(),
            JournalRecordKind::BackgroundCommand { .. } | JournalRecordKind::PublicSnapshot
        ) || matches!(record.kind(), JournalRecordKind::Reconciled { .. })
            && record.context().stamp().is_none()
    }) {
        return Err(JournalError::InvalidBatch);
    }
    driver.database.validate_leaves()?;
    let cipher = driver.cipher.as_ref();
    let db = driver
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let stream = batch.lease().stream();
    let mut catalog = driver.catalog.load(&db, driver.catalog_cipher.as_ref())?;
    driver.catalog.verify_rows(&db, &catalog)?;
    let previous = driver.heads.load(&db, stream)?;
    let mut current = head::required(&db, cipher, stream)?;
    // Lost ACK retries return the original receipt even after writer renewal or
    // deletion; changed semantics still conflict before current revision checks.
    if let Some(command) = batch.command() {
        let existing = receipt(&db, cipher, stream, command.id())?;
        if let CommandAdmission::Duplicate(receipt) = classify_command(
            stream,
            existing.as_ref(),
            command,
            current.position().control(),
        )? {
            return Ok(CommitOutcome::Duplicate(receipt));
        }
    }
    current.require_writer(batch.lease())?;
    if batch.expected() != current.position() {
        return Err(JournalError::Conflict);
    }
    let next = batch.next_position()?;
    let charge = current.budget().appended(batch)?;
    if charge.total() > driver.limits.chat_bytes
        || next
            .records()
            .get()
            .saturating_sub(current.retained().get())
            > driver.limits.chat_records
    {
        return Err(JournalError::Capacity);
    }
    let receipt = JournalCommitReceipt::committed(batch)?;
    records(&db, cipher, batch)?;
    current.set_active(effects::active_after(&db, cipher, &current, batch)?);
    checkpoint(&db, cipher, batch)?;
    outbox(&db, cipher, batch)?;
    if let Some(command) = receipt.command() {
        save_receipt(&db, cipher, command)?;
    }
    let deleted = batch
        .records()
        .iter()
        .any(|record| matches!(record.kind(), JournalRecordKind::Tombstone { .. }));
    current.update(next, deleted, charge);
    current.save(&db, cipher)?;
    driver
        .heads
        .publication(cipher, &driver.catalog, driver.catalog_cipher.as_ref())
        .publish(
            &db,
            &mut catalog,
            &current,
            previous.as_ref(),
            batch.outbox().last().map(ResidentPublication::entry),
        )?;
    db.commit().map_err(storage)?;
    driver.catalog.observe(&catalog)?;
    Ok(CommitOutcome::Committed(Box::new(receipt)))
}

pub(super) fn records(
    db: &Connection,
    cipher: &dyn JournalCipher,
    batch: &JournalBatch,
) -> Result<(), JournalError> {
    for (index, record) in batch.records().iter().enumerate() {
        let sequence = batch.expected().records().advance(index + 1)?;
        if let Some(stamp) = record.context().stamp() {
            super::run_heads::transition(
                db,
                cipher,
                stamp,
                sequence,
                batch.next_position()?.revision(),
            )?;
        }
        effects::apply(db, cipher, batch, record, sequence)?;
        let context = JournalCipherContext::new(
            batch.lease().stream().clone(),
            JournalStorageClass::Record,
            [0; 16],
            sequence,
            batch.next_position()?.revision(),
            batch.lease().epoch(),
        );
        let body = codec::seal(cipher, &context, &codec::record::Record::from(record))?;
        db.execute("INSERT INTO entries(chat,sequence,revision,writer,charge,body) VALUES(?1,?2,?3,?4,?5,?6)", params![head::key(batch.lease().stream()).as_slice(), integer(sequence.get())?, integer(context.revision().get())?, integer(context.writer().get())?, record.budget_bytes() as i64, body]).map_err(storage)?;
    }
    Ok(())
}
fn checkpoint(
    db: &Connection,
    cipher: &dyn JournalCipher,
    batch: &JournalBatch,
) -> Result<(), JournalError> {
    let Some(checkpoint) = batch.checkpoint() else {
        return Ok(());
    };
    let context = head::context(
        batch.lease().stream(),
        JournalStorageClass::Checkpoint,
        [0; 16],
        batch.next_position()?,
        batch.lease().epoch(),
    );
    let body = codec::seal(cipher, &context, &codec::Checkpoint::from(checkpoint))?;
    db.execute("INSERT INTO checkpoints(chat,revision,sequence,writer,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,sequence=excluded.sequence,writer=excluded.writer,body=excluded.body", params![head::key(batch.lease().stream()).as_slice(), integer(context.revision().get())?, integer(context.sequence().get())?, integer(context.writer().get())?, body]).map_err(storage)?;
    Ok(())
}
pub(super) fn outbox(
    db: &Connection,
    cipher: &dyn JournalCipher,
    batch: &JournalBatch,
) -> Result<(), JournalError> {
    let next = batch.next_position()?;
    let material = cipher.chat_key_epoch(batch.lease().stream())?;
    for entry in batch.outbox() {
        use sha2::{Digest, Sha256};
        let anchor = entry.anchor();
        let position = anchor.position();
        let digest: [u8; 32] = Sha256::digest(entry.sealed().as_bytes()).into();
        let index = entry
            .record()
            .get()
            .checked_sub(batch.expected().records().get())
            .and_then(|offset| offset.checked_sub(1))
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or(JournalError::InvalidBatch)?;
        let record = batch
            .records()
            .get(index)
            .ok_or(JournalError::InvalidBatch)?;
        if record
            .context()
            .run()
            .is_some_and(|run| anchor.run() != Some(run))
        {
            return Err(JournalError::InvalidBatch);
        }
        if entry.key_epoch() != material
            || (anchor.kind() == HeadAnchorKind::Snapshot)
                != matches!(record.kind(), JournalRecordKind::PublicSnapshot)
            || position.writer() != batch.lease().epoch()
            || position.revision() != next.revision()
            || position.journal() <= batch.expected().records()
            || position.journal() > next.records()
            || position.event() <= batch.expected().events()
            || position.event() > next.events()
            || anchor.ciphertext().as_bytes() != &digest
            || anchor
                .run()
                .is_some_and(|run| run.stream() != *batch.lease().stream())
        {
            return Err(JournalError::InvalidBatch);
        }
        let context = JournalCipherContext::new(
            batch.lease().stream().clone(),
            JournalStorageClass::Outbox,
            *entry.id().as_bytes(),
            entry.record(),
            batch.next_position()?.revision(),
            batch.lease().epoch(),
        );
        let body = codec::seal(cipher, &context, &codec::Outbox::from(entry.entry()))?;
        db.execute("INSERT INTO outbox(chat,sequence,id,record,revision,writer,body,delivery) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![head::key(batch.lease().stream()).as_slice(), integer(entry.sequence().get())?, entry.id().as_bytes().as_slice(), integer(entry.record().get())?, integer(context.revision().get())?, integer(context.writer().get())?, body, entry.delivery_id().as_bytes().as_slice()]).map_err(storage)?;
    }
    Ok(())
}
pub(super) fn save_receipt(
    db: &Connection,
    cipher: &dyn JournalCipher,
    receipt: &CommandReceipt,
) -> Result<(), JournalError> {
    let context = head::context(
        receipt.lease().stream(),
        JournalStorageClass::CommandReceipt,
        *receipt.acceptance().id().as_bytes(),
        receipt.position(),
        receipt.lease().epoch(),
    );
    let body = codec::seal(cipher, &context, &codec::Receipt::from(receipt))?;
    db.execute(
        "INSERT INTO commands(chat,id,revision,sequence,writer,body) VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            head::key(receipt.lease().stream()).as_slice(),
            receipt.acceptance().id().as_bytes().as_slice(),
            integer(context.revision().get())?,
            integer(context.sequence().get())?,
            integer(context.writer().get())?,
            body
        ],
    )
    .map_err(storage)?;
    Ok(())
}
pub(super) fn receipt(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    id: CommandId,
) -> Result<Option<CommandReceipt>, JournalError> {
    let row = db
        .query_row(
            "SELECT revision,sequence,writer,length(body) FROM commands WHERE chat=?1 AND id=?2",
            params![head::key(stream).as_slice(), id.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(storage)?;
    let Some((revision, sequence, writer, length)) = row else {
        return Ok(None);
    };
    head::check_length(length)?;
    let context = JournalCipherContext::new(
        stream.clone(),
        JournalStorageClass::CommandReceipt,
        *id.as_bytes(),
        JournalSequence::from_u64(unsigned(sequence)?),
        StreamRevision::from_u64(unsigned(revision)?),
        WriterEpoch::new(unsigned(writer)?)?,
    );
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM commands WHERE chat=?1 AND id=?2",
            params![head::key(stream).as_slice(), id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let receipt = codec::open::<codec::Receipt>(cipher, &context, &body)?.restore()?;
    if receipt.acceptance().id() != id
        || receipt.lease().stream() != stream
        || receipt.lease().epoch() != context.writer()
        || receipt.position().revision() != context.revision()
        || receipt.position().records() != context.sequence()
    {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(receipt))
}
