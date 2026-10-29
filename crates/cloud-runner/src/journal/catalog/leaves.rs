//! Ordered bounded descriptor commitment; only selected cells materialize bodies.
use super::{head, integer, storage, unsigned};
use crate::journal::metadata;
use nocterm_cloud_model::{identity::CloudChatId, journal::*};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(super) fn root(
    db: &Connection,
    binding: &JournalCatalogBinding,
    chats: &BTreeSet<[u8; 16]>,
    capacity: u64,
) -> Result<[u8; 32], JournalError> {
    metadata::rows(db, capacity)?;
    let mut hash = Sha256::new();
    hash.update(b"nocterm/native-cloud-current-head-leaves/v2");
    hash.update(head::scope_key(binding.scope()));
    hash.update(binding.runner().as_bytes());
    hash.update(binding.epoch().get().to_be_bytes());
    let mut expected = chats.iter();
    let mut statement = db.prepare(
        "SELECT c.chat,c.binding,c.scope,c.chat_id,c.runner,c.key_epoch,h.revision,h.writer,length(h.body),length(h.digest),h.digest,c.writer,c.revision,c.records,c.events,c.control,c.retained,c.deleted,c.charge,length(c.body),c.header_digest FROM chats c LEFT JOIN current_heads h ON h.chat=c.chat ORDER BY c.chat_id LIMIT ?1"
    ).map_err(storage)?;
    let mut rows = statement.query([integer(capacity + 1)?]).map_err(storage)?;
    let mut count = 0u64;
    while let Some(row) = rows.next().map_err(storage)? {
        count += 1;
        if count > capacity {
            return Err(JournalError::Capacity);
        }
        let chat: [u8; 16] = blob(row, 3)?
            .try_into()
            .map_err(|_| JournalError::Corrupt)?;
        if expected.next() != Some(&chat) {
            return Err(JournalError::Corrupt);
        }
        let stream = binding.stream(CloudChatId::from_bytes(chat));
        if blob(row, 0)? != head::key(&stream)
            || blob(row, 1)? != head::binding_key(&stream)
            || blob(row, 2)? != head::scope_key(binding.scope())
            || blob(row, 4)? != binding.runner().as_bytes()
            || unsigned(row.get(5).map_err(storage)?)? != binding.epoch().get()
        {
            return Err(JournalError::Corrupt);
        }
        let revision = unsigned(row.get(6).map_err(storage)?)?;
        let writer: Option<i64> = row.get(7).map_err(storage)?;
        let writer = writer.map(unsigned).transpose()?;
        let length = unsigned(row.get(8).map_err(storage)?)?;
        let digest_length: i64 = row.get(9).map_err(storage)?;
        let digest = blob(row, 10)?;
        if revision == 0
            || writer == Some(0)
            || length == 0
            || length > MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT as u64
            || digest_length != 32
            || digest.len() != 32
        {
            return Err(JournalError::Corrupt);
        }
        hash.update(head::key(&stream));
        hash.update(head::binding_key(&stream));
        hash.update(chat);
        hash.update(revision.to_be_bytes());
        hash.update([u8::from(writer.is_some())]);
        if let Some(writer) = writer {
            hash.update(writer.to_be_bytes());
        }
        hash.update(length.to_be_bytes());
        hash.update(digest);
        // Protect the native Header's queryable footprint and ciphertext link,
        // independently from the root-cell descriptor. A partial root restore
        // cannot hide a newer Header even without its content key.
        for column in 11..=19 {
            let value = unsigned(row.get(column).map_err(storage)?)?;
            hash.update(value.to_be_bytes());
        }
        hash.update(blob(row, 20)?);
    }
    if expected.next().is_some() {
        return Err(JournalError::Corrupt);
    }
    Ok(hash.finalize().into())
}
fn blob<'a>(row: &'a rusqlite::Row<'_>, column: usize) -> Result<&'a [u8], JournalError> {
    row.get_ref(column)
        .map_err(storage)?
        .as_blob()
        .map_err(|_| JournalError::Corrupt)
}
