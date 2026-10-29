//! Root-protected current observations survive content-key retirement.
mod publication;
use super::{
    catalog::CatalogOwner, head, header_link::HeaderLink, integer, metadata, storage, unsigned,
};
use crate::codec;
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(super) struct HeadCells {
    cipher: Arc<dyn JournalChatHeadCipher>,
    marks: Arc<Mutex<BTreeMap<StreamBinding, CellMark>>>,
    capacity: usize,
}
#[derive(Clone)]
struct CellMark {
    revision: u64,
    writer: Option<WriterEpoch>,
    control: ChatControlRevision,
    deleted: bool,
    material: ChatKeyEpoch,
}
pub(super) struct Cell {
    revision: u64,
    value: CurrentChatHead,
    header: HeaderLink,
}
impl Cell {
    pub(super) fn value(&self) -> &CurrentChatHead {
        &self.value
    }
}
impl HeadCells {
    pub(super) fn new(cipher: Arc<dyn JournalChatHeadCipher>, capacity: u64) -> Self {
        Self {
            cipher,
            marks: Arc::new(Mutex::new(BTreeMap::new())),
            capacity: capacity as usize,
        }
    }
    pub(super) fn load(
        &self,
        db: &Connection,
        stream: &StreamBinding,
    ) -> Result<Option<Cell>, JournalError> {
        metadata::require_transaction(db)?;
        // The caller's complete metadata pass has bounded every lookup key,
        // scalar and BLOB in this same snapshot before this selected access.
        let row = db
            .query_row(
                "SELECT revision,writer,digest,body FROM current_heads WHERE chat=?1",
                [head::key(stream).as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(storage)?;
        let Some((revision, writer, digest, sealed)) = row else {
            return Ok(None);
        };
        let revision = unsigned(revision)?;
        let writer = writer
            .map(|writer| WriterEpoch::new(unsigned(writer)?).map_err(JournalError::from))
            .transpose()?;
        if digest.as_slice() != Sha256::digest(&sealed).as_slice() {
            return Err(JournalError::Corrupt);
        }
        let context = JournalChatHeadContext::new(stream.clone(), writer, revision)?;
        let plain = Zeroizing::new(self.cipher.open(&context, &sealed)?);
        let (value, header) = codec::head::decode(&plain)?;
        if value.stream() != stream || value.facts().writer != writer {
            return Err(JournalError::Corrupt);
        }
        let actual = HeaderLink::read(db, stream)?;
        if header != actual {
            return Err(JournalError::Corrupt);
        }
        header.check_observation(&value)?;
        Ok(Some(Cell {
            revision,
            value,
            header,
        }))
    }
    pub(super) fn required(
        &self,
        db: &Connection,
        stream: &StreamBinding,
    ) -> Result<Cell, JournalError> {
        self.load(db, stream)?.ok_or(JournalError::Corrupt)
    }
    pub(super) fn publication<'a>(
        &'a self,
        content: &'a dyn JournalCipher,
        catalog_owner: &'a CatalogOwner,
        catalog_cipher: &'a dyn JournalCatalogCipher,
    ) -> publication::HeadPublication<'a> {
        publication::HeadPublication::new(self, content, catalog_owner, catalog_cipher)
    }
    fn store(
        &self,
        db: &Connection,
        revision: u64,
        value: &CurrentChatHead,
        header: &HeaderLink,
    ) -> Result<(), JournalError> {
        let stream = value.stream();
        let context = JournalChatHeadContext::new(stream.clone(), value.facts().writer, revision)?;
        let plain = Zeroizing::new(codec::head::encode(value, header)?);
        let sealed = self.cipher.seal(&context, &plain)?;
        if sealed.is_empty() || sealed.len() > MAX_JOURNAL_CHAT_HEAD_CIPHERTEXT {
            return Err(JournalError::Capacity);
        }
        let digest: [u8; 32] = Sha256::digest(&sealed).into();
        db.execute("INSERT INTO current_heads(chat,revision,writer,digest,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(chat) DO UPDATE SET revision=excluded.revision,writer=excluded.writer,digest=excluded.digest,body=excluded.body", params![head::key(stream).as_slice(), integer(revision)?, value.facts().writer.map(|writer| integer(writer.get())).transpose()?, digest.as_slice(), sealed]).map_err(storage)?;
        Ok(())
    }
    fn check_progress(&self, old: &Cell, next: &CurrentChatHead) -> Result<(), JournalError> {
        let before = old.value.facts();
        let after = next.facts();
        if after.writer < before.writer
            || after.control < before.control
            || (before.deleted && !after.deleted)
            || old.value.key_epoch() != next.key_epoch()
            || before.head.as_ref().is_some_and(|head| {
                after
                    .head
                    .as_ref()
                    .is_none_or(|next| next.position().event() < head.position().event())
            })
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    pub(super) fn observe(&self, stream: &StreamBinding, cell: &Cell) -> Result<(), JournalError> {
        let facts = cell.value.facts();
        let next = CellMark {
            revision: cell.revision,
            writer: facts.writer,
            control: facts.control,
            deleted: facts.deleted,
            material: cell.value.key_epoch(),
        };
        let mut marks = self.marks.lock();
        if let Some(previous) = marks.get(stream) {
            if next.revision < previous.revision
                || next.writer < previous.writer
                || next.control < previous.control
                || (previous.deleted && !next.deleted)
                || next.material < previous.material
            {
                return Err(JournalError::Corrupt);
            }
        } else if marks.len() >= self.capacity {
            return Err(JournalError::Capacity);
        }
        marks.insert(stream.clone(), next);
        Ok(())
    }
    pub(super) fn check_content(
        &self,
        current: &head::Head,
        cell: &Cell,
    ) -> Result<(), JournalError> {
        let facts = cell.value.facts();
        if facts.writer != Some(current.lease().epoch())
            || facts.control != current.position().control()
            || facts.deleted != current.deleted()
            || facts
                .head
                .as_ref()
                .map_or(EventSequence::default(), |head| head.position().event())
                != current.position().events()
            || facts.head.is_none() != (current.position().events().get() == 0)
            || facts.head.as_ref().is_some_and(|head| {
                head.position().journal() > current.position().records()
                    || head.position().event() > current.position().events()
                    || head.position().revision() > current.position().revision()
            })
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
}
fn equal_facts(left: &CurrentHeadFacts, right: &CurrentHeadFacts) -> bool {
    left.writer == right.writer
        && left.control == right.control
        && left.deleted == right.deleted
        && left.head == right.head
        && left.event_floor == right.event_floor
        && left.snapshot == right.snapshot
}
