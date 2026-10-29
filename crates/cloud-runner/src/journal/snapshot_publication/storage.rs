//! Separate bounded payloads, cross-linked by the protected native Header.
use super::*;
use crate::codec as native;
use nocterm_core::allocation::AllocationLease;
use rusqlite::OptionalExtension;

pub(super) fn reserve_decode(
    work: &dyn PublicationWorkBudget,
    envelope: usize,
) -> Result<AllocationLease, JournalError> {
    // Raw SQLite envelope + AEAD plaintext (never larger than ciphertext) +
    // decoded owned bytes (bounded by plaintext), plus bounded typed metadata.
    let raw = envelope;
    let plaintext = envelope;
    let owned_and_native_write = ResidentPublication::required_bytes(envelope)?;
    let bytes = raw
        .checked_add(plaintext)
        .and_then(|n| n.checked_add(owned_and_native_write))
        .ok_or(JournalError::Capacity)?;
    work.try_reserve(PublicationAllocation::new(bytes, 1)?)
}
pub(in crate::journal) fn load(
    db: &Connection,
    cipher: &dyn JournalCipher,
    head: &head::Head,
) -> Result<codec::Transfer, JournalError> {
    let row = db
        .query_row(
            "SELECT revision,sequence,writer,length(body) FROM snapshot_transfers WHERE chat=?1",
            [head::key(head.lease().stream()).as_slice()],
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
        .map_err(sql_error)?;
    let Some((revision, sequence, writer, length)) = row else {
        if head.transfer().is_some() {
            return Err(JournalError::Corrupt);
        }
        return Ok(codec::Transfer {
            pending: None,
            active: None,
        });
    };
    if !(1..=16 * 1024).contains(&length) {
        return Err(JournalError::Corrupt);
    }
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM snapshot_transfers WHERE chat=?1",
            [head::key(head.lease().stream()).as_slice()],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if head.transfer() != Some(Sha256::digest(&body).into()) {
        return Err(JournalError::Corrupt);
    }
    let context = JournalCipherContext::new(
        head.lease().stream().clone(),
        JournalStorageClass::Snapshot,
        [0xf0; 16],
        JournalSequence::from_u64(super::super::unsigned(sequence)?),
        StreamRevision::from_u64(super::super::unsigned(revision)?),
        WriterEpoch::new(super::super::unsigned(writer)?)?,
    );
    native::open(cipher, &context, &body)
}
pub(super) fn save(
    db: &Connection,
    cipher: &dyn JournalCipher,
    head: &mut head::Head,
    state: &codec::Transfer,
) -> Result<(), JournalError> {
    let context = head::context(
        head.lease().stream(),
        JournalStorageClass::Snapshot,
        [0xf0; 16],
        head.position(),
        head.lease().epoch(),
    );
    let body = native::seal(cipher, &context, state)?;
    if body.len() > 16 * 1024 {
        return Err(JournalError::Capacity);
    }
    head.set_transfer(Sha256::digest(&body).into());
    db.execute("INSERT INTO snapshot_transfers(chat,revision,sequence,writer,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,sequence=excluded.sequence,writer=excluded.writer,body=excluded.body", params![head::key(head.lease().stream()).as_slice(),integer(context.revision().get())?,integer(context.sequence().get())?,integer(context.writer().get())?,body]).map_err(sql_error)?;
    Ok(())
}
pub(super) fn put<T: serde::Serialize>(
    db: &Connection,
    cipher: &dyn JournalCipher,
    candidate: &SnapshotCandidate,
    kind: u8,
    value: &T,
) -> Result<(), JournalError> {
    let mut id = *candidate.delivery_id().as_bytes();
    id[0] ^= kind;
    let context = JournalCipherContext::new(
        candidate.stream().clone(),
        JournalStorageClass::Snapshot,
        id,
        candidate.anchor().position().journal(),
        candidate.anchor().position().revision(),
        candidate.anchor().position().writer(),
    );
    let body = native::seal(cipher, &context, value)?;
    db.execute("INSERT INTO snapshot_payloads(chat,kind,revision,sequence,writer,body) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(chat,kind) DO UPDATE SET revision=excluded.revision,sequence=excluded.sequence,writer=excluded.writer,body=excluded.body",params![head::key(candidate.stream()).as_slice(),kind,integer(context.revision().get())?,integer(context.sequence().get())?,integer(context.writer().get())?,body]).map_err(sql_error)?;
    Ok(())
}
pub(super) fn get<T: serde::de::DeserializeOwned>(
    db: &Connection,
    cipher: &dyn JournalCipher,
    candidate: &SnapshotCandidate,
    kind: u8,
    work: &dyn PublicationWorkBudget,
) -> Result<(T, AllocationLease), JournalError> {
    let length: i64 = db
        .query_row(
            "SELECT length(body) FROM snapshot_payloads WHERE chat=?1 AND kind=?2",
            params![head::key(candidate.stream()).as_slice(), kind],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let length = head::check_length(length)?;
    let allocation = reserve_decode(work, length)?;
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM snapshot_payloads WHERE chat=?1 AND kind=?2",
            params![head::key(candidate.stream()).as_slice(), kind],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let mut id = *candidate.delivery_id().as_bytes();
    id[0] ^= if kind == 3 { 2 } else { kind };
    let context = JournalCipherContext::new(
        candidate.stream().clone(),
        JournalStorageClass::Snapshot,
        id,
        candidate.anchor().position().journal(),
        candidate.anchor().position().revision(),
        candidate.anchor().position().writer(),
    );
    Ok((native::open(cipher, &context, &body)?, allocation))
}
pub(super) fn outbox_length(
    db: &Connection,
    stream: &StreamBinding,
    next: EventSequence,
) -> Result<usize, JournalError> {
    // The complete TYPEOF pass must precede even length(body): SQLite's BLOB
    // affinity permits TEXT, whose length() materializes the whole value. The
    // shared guard checks every routing/scalar field in this same transaction.
    super::super::metadata::outbox_rows(db, MAX_JOURNAL_CATALOG_CHATS as u64 * 1_000_001)?;
    head::check_length(
        db.query_row(
            "SELECT length(body) FROM outbox WHERE chat=?1 AND sequence=?2",
            params![head::key(stream).as_slice(), integer(next.get())?],
            |row| row.get(0),
        )
        .map_err(sql_error)?,
    )
}
