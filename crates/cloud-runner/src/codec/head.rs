//! Bounded root-protected native observations, independent from content archives.
use super::identity::{Run, Stream};
use crate::journal::header_link::HeaderLink;
use nocterm_cloud_model::{identity::*, journal::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Anchor {
    kind: u8,
    writer: u64,
    event: u64,
    journal: u64,
    revision: u64,
    id: [u8; 16],
    run: Option<Run>,
    ciphertext: [u8; 32],
}
impl From<&HeadAnchor> for Anchor {
    fn from(value: &HeadAnchor) -> Self {
        let position = value.position();
        Self {
            kind: match value.kind() {
                HeadAnchorKind::Event => 1,
                HeadAnchorKind::Snapshot => 2,
            },
            writer: position.writer().get(),
            event: position.event().get(),
            journal: position.journal().get(),
            revision: position.revision().get(),
            id: *value.id().as_bytes(),
            run: value.run().map(Run::from),
            ciphertext: *value.ciphertext().as_bytes(),
        }
    }
}
impl Anchor {
    pub(crate) fn restore(self) -> Result<HeadAnchor, JournalError> {
        let kind = match self.kind {
            1 => HeadAnchorKind::Event,
            2 => HeadAnchorKind::Snapshot,
            _ => return Err(JournalError::Corrupt),
        };
        Ok(HeadAnchor::new(
            kind,
            HeadRecordPosition::new(
                WriterEpoch::new(self.writer)?,
                EventSequence::from_u64(self.event),
                JournalSequence::from_u64(self.journal),
                StreamRevision::from_u64(self.revision),
            )?,
            EventId::from_bytes(self.id),
            self.run.map(Run::restore).transpose()?,
            Fingerprint::from_bytes(self.ciphertext),
        ))
    }
}
#[derive(Serialize, Deserialize)]
struct StoredHead {
    stream: Stream,
    material: u64,
    writer: Option<u64>,
    control: u64,
    deleted: bool,
    head: Option<Anchor>,
    floor: u64,
    snapshot: Option<Anchor>,
    header: HeaderLink,
}
impl StoredHead {
    fn new(value: &CurrentChatHead, header: HeaderLink) -> Self {
        let facts = value.facts();
        Self {
            stream: value.stream().into(),
            material: value.key_epoch().get(),
            writer: facts.writer.map(WriterEpoch::get),
            control: facts.control.get(),
            deleted: facts.deleted,
            head: facts.head.as_ref().map(Anchor::from),
            floor: facts.event_floor.get(),
            snapshot: facts.snapshot.as_ref().map(Anchor::from),
            header,
        }
    }
}
impl StoredHead {
    fn restore(self) -> Result<(CurrentChatHead, HeaderLink), JournalError> {
        self.header.validate()?;
        let value = CurrentChatHead::new(
            self.stream.restore()?,
            ChatKeyEpoch::new(self.material)?,
            CurrentHeadFacts {
                writer: self.writer.map(WriterEpoch::new).transpose()?,
                control: ChatControlRevision::from_u64(self.control),
                deleted: self.deleted,
                head: self.head.map(Anchor::restore).transpose()?,
                event_floor: EventSequence::from_u64(self.floor),
                snapshot: self.snapshot.map(Anchor::restore).transpose()?,
            },
        )?;
        Ok((value, self.header))
    }
}
pub(crate) fn encode(
    value: &CurrentChatHead,
    header: &HeaderLink,
) -> Result<Vec<u8>, JournalError> {
    let bytes = postcard::to_allocvec(&(2u8, StoredHead::new(value, header.clone())))
        .map_err(|_| JournalError::Corrupt)?;
    if bytes.is_empty() || bytes.len() > MAX_JOURNAL_CHAT_HEAD_PLAINTEXT {
        return Err(JournalError::Capacity);
    }
    Ok(bytes)
}
pub(crate) fn decode(bytes: &[u8]) -> Result<(CurrentChatHead, HeaderLink), JournalError> {
    if bytes.is_empty() || bytes.len() > MAX_JOURNAL_CHAT_HEAD_PLAINTEXT {
        return Err(JournalError::Corrupt);
    }
    let (version, _) = postcard::take_from_bytes::<u8>(bytes).map_err(|_| JournalError::Corrupt)?;
    if version != 2 {
        return Err(JournalError::UnsupportedFormat);
    }
    let ((version, value), trailing) =
        postcard::take_from_bytes::<(u8, StoredHead)>(bytes).map_err(|_| JournalError::Corrupt)?;
    if version != 2 || !trailing.is_empty() {
        return Err(JournalError::UnsupportedFormat);
    }
    value.restore()
}
