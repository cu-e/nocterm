use super::{Driver, codec, head, integer, storage, unsigned};
use nocterm_cloud_model::{
    authority::StreamBinding, engine::EngineCheckpoint, identity::*, journal::*,
};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn read(
    driver: &mut Driver,
    stream: &StreamBinding,
    after: JournalSequence,
    limits: JournalReadLimits,
) -> Result<JournalPage, JournalError> {
    driver.database.validate_leaves()?;
    let db = driver.connection.transaction().map_err(storage)?;
    let head = head::required(&db, driver.cipher.as_ref(), stream)?;
    read_from(&db, driver.cipher.as_ref(), stream, &head, after, limits)
}
pub(super) fn read_from(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    head: &head::Head,
    after: JournalSequence,
    limits: JournalReadLimits,
) -> Result<JournalPage, JournalError> {
    if after < head.retained() || after > head.position().records() {
        return Err(JournalError::Gap {
            expected: after,
            actual: head.retained(),
        });
    }
    let mut statement = db.prepare("SELECT sequence,revision,writer,charge,length(body) FROM entries WHERE chat=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(storage)?;
    let mut rows = statement
        .query(params![
            head::key(stream).as_slice(),
            integer(after.get())?,
            limits.records() as i64
        ])
        .map_err(storage)?;
    let mut records = Vec::new();
    let mut charged = 0usize;
    while let Some(row) = rows.next().map_err(storage)? {
        let sequence = JournalSequence::from_u64(unsigned(row.get(0).map_err(storage)?)?);
        let revision = StreamRevision::from_u64(unsigned(row.get(1).map_err(storage)?)?);
        let writer = WriterEpoch::new(unsigned(row.get(2).map_err(storage)?)?)?;
        let charge = usize::try_from(row.get::<_, i64>(3).map_err(storage)?)
            .map_err(|_| JournalError::Corrupt)?;
        let length = head::check_length(row.get(4).map_err(storage)?)?;
        if length > charge {
            return Err(JournalError::Corrupt);
        }
        if charge > limits.bytes().saturating_sub(charged) {
            if records.is_empty() {
                return Err(JournalError::Capacity);
            }
            break;
        }
        let context = JournalCipherContext::new(
            stream.clone(),
            JournalStorageClass::Record,
            [0; 16],
            sequence,
            revision,
            writer,
        );
        let body: Vec<u8> = db
            .query_row(
                "SELECT body FROM entries WHERE chat=?1 AND sequence=?2",
                params![head::key(stream).as_slice(), integer(sequence.get())?],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let record = codec::open::<codec::record::Record>(cipher, &context, &body)?.restore()?;
        if charge != record.budget_bytes() {
            return Err(JournalError::Corrupt);
        }
        charged += charge;
        records.push(StoredRecord::new(
            stream.clone(),
            sequence,
            revision,
            record,
        )?);
    }
    JournalPage::new(stream.clone(), after, head.position(), records, limits)
}

pub(super) fn checkpoint(
    driver: &mut Driver,
    stream: &StreamBinding,
) -> Result<Option<EngineCheckpoint>, JournalError> {
    driver.database.validate_leaves()?;
    let db = driver.connection.transaction().map_err(storage)?;
    let head = head::required(&db, driver.cipher.as_ref(), stream)?;
    let Some((context, body)) = object(
        &db,
        stream,
        JournalStorageClass::Checkpoint,
        head.position(),
    )?
    else {
        return Ok(None);
    };
    let checkpoint =
        codec::open::<codec::Checkpoint>(driver.cipher.as_ref(), &context, &body)?.restore()?;
    if checkpoint.run().stream() != *stream || checkpoint.journal_revision() != context.revision() {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(checkpoint))
}
pub(super) fn snapshot(
    driver: &mut Driver,
    stream: &StreamBinding,
) -> Result<Option<JournalSnapshot>, JournalError> {
    driver.database.validate_leaves()?;
    let db = driver.connection.transaction().map_err(storage)?;
    let head = head::required(&db, driver.cipher.as_ref(), stream)?;
    let Some((context, body)) =
        object(&db, stream, JournalStorageClass::Snapshot, head.position())?
    else {
        return Ok(None);
    };
    let snapshot =
        codec::open::<codec::Snapshot>(driver.cipher.as_ref(), &context, &body)?.restore()?;
    if snapshot.stream() != stream
        || snapshot.at().revision() != context.revision()
        || snapshot.at().records() != context.sequence()
    {
        return Err(JournalError::Corrupt);
    }
    Ok(Some(snapshot))
}

pub(super) fn object(
    db: &Connection,
    stream: &StreamBinding,
    class: JournalStorageClass,
    head: JournalPosition,
) -> Result<Option<(JournalCipherContext, Vec<u8>)>, JournalError> {
    let (header_sql, body_sql) = match class {
        JournalStorageClass::Checkpoint => (
            "SELECT revision,sequence,writer,length(body) FROM checkpoints WHERE chat=?1",
            "SELECT body FROM checkpoints WHERE chat=?1",
        ),
        JournalStorageClass::Snapshot => (
            "SELECT revision,sequence,writer,length(body) FROM snapshots WHERE chat=?1",
            "SELECT body FROM snapshots WHERE chat=?1",
        ),
        _ => return Err(JournalError::Corrupt),
    };
    let row = db
        .query_row(header_sql, [head::key(stream).as_slice()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .optional()
        .map_err(storage)?;
    let Some((revision, sequence, writer, length)) = row else {
        return Ok(None);
    };
    head::check_length(length)?;
    let sequence = JournalSequence::from_u64(unsigned(sequence)?);
    let revision = StreamRevision::from_u64(unsigned(revision)?);
    if sequence > head.records() || revision > head.revision() {
        return Err(JournalError::Corrupt);
    }
    let context = JournalCipherContext::new(
        stream.clone(),
        class,
        [0; 16],
        sequence,
        revision,
        WriterEpoch::new(unsigned(writer)?)?,
    );
    let body: Vec<u8> = db
        .query_row(body_sql, [head::key(stream).as_slice()], |row| row.get(0))
        .map_err(storage)?;
    Ok(Some((context, body)))
}

pub(super) fn outbox(
    driver: &mut Driver,
    stream: &StreamBinding,
    after: EventSequence,
    limits: JournalReadLimits,
) -> Result<Vec<OutboxEntry>, JournalError> {
    driver.database.validate_leaves()?;
    let db = driver.connection.transaction().map_err(storage)?;
    let head = head::required(&db, driver.cipher.as_ref(), stream)?;
    outbox_from(&db, driver.cipher.as_ref(), stream, &head, after, limits)
}
pub(super) fn outbox_from(
    db: &Connection,
    cipher: &dyn JournalCipher,
    stream: &StreamBinding,
    head: &head::Head,
    after: EventSequence,
    limits: JournalReadLimits,
) -> Result<Vec<OutboxEntry>, JournalError> {
    super::metadata::outbox_rows(db, MAX_JOURNAL_CATALOG_CHATS as u64 * 1_000_001)?;
    if after < head.event_floor() || after > head.position().events() {
        return Err(JournalError::EventGap {
            expected: after,
            actual: head.event_floor(),
        });
    }
    let mut statement = db.prepare("SELECT sequence,id,record,revision,writer,length(body) FROM outbox WHERE chat=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(storage)?;
    let mut rows = statement
        .query(params![
            head::key(stream).as_slice(),
            integer(after.get())?,
            limits.records() as i64
        ])
        .map_err(storage)?;
    let mut entries = Vec::new();
    let mut charged = 0usize;
    while let Some(row) = rows.next().map_err(storage)? {
        let length = head::check_length(row.get(5).map_err(storage)?)?;
        if length > limits.bytes().saturating_sub(charged) {
            if entries.is_empty() {
                return Err(JournalError::Capacity);
            }
            break;
        }
        let sequence = EventSequence::from_u64(unsigned(row.get(0).map_err(storage)?)?);
        let id: [u8; 16] = row
            .get::<_, Vec<u8>>(1)
            .map_err(storage)?
            .try_into()
            .map_err(|_| JournalError::Corrupt)?;
        let record = JournalSequence::from_u64(unsigned(row.get(2).map_err(storage)?)?);
        let revision = StreamRevision::from_u64(unsigned(row.get(3).map_err(storage)?)?);
        let writer = WriterEpoch::new(unsigned(row.get(4).map_err(storage)?)?)?;
        if sequence != after.advance(entries.len() + 1)?
            || record > head.position().records()
            || revision > head.position().revision()
        {
            return Err(JournalError::Corrupt);
        }
        let context = JournalCipherContext::new(
            stream.clone(),
            JournalStorageClass::Outbox,
            id,
            record,
            revision,
            writer,
        );
        let body: Vec<u8> = db
            .query_row(
                "SELECT body FROM outbox WHERE chat=?1 AND sequence=?2",
                params![head::key(stream).as_slice(), integer(sequence.get())?],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let entry = codec::open::<codec::Outbox>(cipher, &context, &body)?.restore()?;
        if entry.id().as_bytes() != &id || entry.sequence() != sequence || entry.record() != record
        {
            return Err(JournalError::Corrupt);
        }
        charged += length;
        entries.push(entry);
    }
    if entries.is_empty() && after != head.position().events() {
        return Err(JournalError::EventGap {
            expected: after.advance(1)?,
            actual: head.position().events(),
        });
    }
    Ok(entries)
}
