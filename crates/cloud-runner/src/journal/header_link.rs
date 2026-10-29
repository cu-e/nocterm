//! Protected cross-store commitment, verifiable without retaining a content key.
use super::{head, metadata, storage, unsigned};
use nocterm_cloud_model::{authority::StreamBinding, identity::*, journal::*};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const MAX_HEADER_CIPHERTEXT: usize = 16 * 1024;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HeaderLink {
    writer: u64,
    revision: u64,
    records: u64,
    events: u64,
    control: u64,
    retained: u64,
    deleted: bool,
    charge: u64,
    length: u64,
    digest: [u8; 32],
}
impl HeaderLink {
    /// Callers have completed the bounded metadata type/length passes in this
    /// transaction before any lookup/sort. Trusted mutations preserve shapes.
    pub(super) fn read(db: &Connection, stream: &StreamBinding) -> Result<Self, JournalError> {
        metadata::require_transaction(db)?;
        let value = db.query_row(
            "SELECT writer,revision,records,events,control,retained,deleted,charge,length(body),header_digest FROM chats WHERE chat=?1",
            [head::key(stream).as_slice()],
            |row| Ok((row.get::<_,i64>(0)?, row.get::<_,i64>(1)?, row.get::<_,i64>(2)?, row.get::<_,i64>(3)?, row.get::<_,i64>(4)?, row.get::<_,i64>(5)?, row.get::<_,bool>(6)?, row.get::<_,i64>(7)?, row.get::<_,i64>(8)?, row.get::<_,Vec<u8>>(9)?)),
        ).map_err(storage)?;
        let result = Self {
            writer: unsigned(value.0)?,
            revision: unsigned(value.1)?,
            records: unsigned(value.2)?,
            events: unsigned(value.3)?,
            control: unsigned(value.4)?,
            retained: unsigned(value.5)?,
            deleted: value.6,
            charge: unsigned(value.7)?,
            length: unsigned(value.8)?,
            digest: value.9.try_into().map_err(|_| JournalError::Corrupt)?,
        };
        result.validate()?;
        let body: Vec<u8> = db
            .query_row(
                "SELECT body FROM chats WHERE chat=?1",
                params![head::key(stream).as_slice()],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let digest: [u8; 32] = Sha256::digest(&body).into();
        if body.len() as u64 != result.length || digest != result.digest {
            return Err(JournalError::Corrupt);
        }
        Ok(result)
    }
    pub(crate) fn validate(&self) -> Result<(), JournalError> {
        if self.writer == 0
            || self.control > self.revision
            || self.revision > self.records
            || self.retained > self.records
            || self.length == 0
            || self.length > MAX_HEADER_CIPHERTEXT as u64
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    pub(super) fn check_observation(&self, value: &CurrentChatHead) -> Result<(), JournalError> {
        self.validate()?;
        let facts = value.facts();
        let tail = facts.head.as_ref();
        if self.retained > 0
            && facts.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.position().journal().get() != self.retained + 1
                    || snapshot.position().event() != facts.event_floor
            })
        {
            return Err(JournalError::Corrupt);
        }
        if facts.writer.map(WriterEpoch::get) != Some(self.writer)
            || facts.control.get() != self.control
            || facts.deleted != self.deleted
            || tail.map_or(0, |anchor| anchor.position().event().get()) != self.events
            || (tail.is_none() != (self.events == 0))
            || tail.is_some_and(|anchor| {
                anchor.position().journal().get() > self.records
                    || anchor.position().revision().get() > self.revision
            })
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
}
