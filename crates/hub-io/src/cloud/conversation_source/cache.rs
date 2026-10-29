//! One bounded verified raw page and known-head marker, never a fake snapshot.
use super::{
    outbox::{Input, field},
    storage::SealedCell,
};
use nocterm_cloud_model::{
    identity::*,
    projection::{MAX_REPLAY_EVENTS, ProjectionCursor, ProjectionError},
};
use nocterm_core::fs::{AtomicPublication, open_private_regular, require_private_regular};
use nocterm_hub_model::cloud::{
    ChatCoordinate, JournalClass, JournalCoordinate,
    relay::{MAX_RELAY_BINDING_BYTES, PublicationMetadata, RelayBinding},
};
use nocterm_keys::cloud::ChatKey;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
};
use zeroize::Zeroizing;
const MAGIC: &[u8; 8] = b"NCDVC001";
const PLAIN: &[u8; 8] = b"NCDVP001";

pub(super) struct CachedPublication {
    pub metadata: PublicationMetadata,
    pub delivery: u64,
    pub ciphertext: Vec<u8>,
}
#[derive(Clone)]
pub(super) struct CacheMarker {
    pub writer: WriterEpoch,
    pub known: ProjectionCursor,
    pub control: ChatControlRevision,
    pub deleted: bool,
    pub catalog_revision: u64,
    pub catalog_root: Fingerprint,
    pub delivered: u64,
}
pub(super) struct CacheFile {
    path: PathBuf,
    parent: File,
    maximum: usize,
}
impl CacheFile {
    pub(super) fn new(root: PathBuf, maximum: usize) -> Result<Self, ProjectionError> {
        Ok(Self {
            path: root.join("verified-page.v1"),
            parent: File::open(root).map_err(|_| ProjectionError::Persistence)?,
            maximum,
        })
    }
    pub(super) fn read(&self) -> Result<Option<SealedCell>, ProjectionError> {
        let mut file = match open_private_regular(&self.path) {
            Ok(file) => file,
            Err(error) if error.io_kind() == Some(io::ErrorKind::NotFound) => return Ok(None),
            Err(_) => return Err(ProjectionError::Persistence),
        };
        require_private_regular(&file).map_err(|_| ProjectionError::Persistence)?;
        let size = usize::try_from(
            file.metadata()
                .map_err(|_| ProjectionError::Persistence)?
                .len(),
        )
        .map_err(|_| ProjectionError::Capacity)?;
        if size < 16 + 42 || size > self.maximum + 16 {
            return Err(ProjectionError::Capacity);
        }
        let mut header = [0; 16];
        file.read_exact(&mut header)
            .map_err(|_| ProjectionError::Persistence)?;
        if &header[..8] != MAGIC {
            return Err(ProjectionError::InvalidRecord);
        }
        let writer = u64::from_be_bytes(
            header[8..]
                .try_into()
                .map_err(|_| ProjectionError::InvalidRecord)?,
        );
        let mut ciphertext = vec![0; size - 16];
        file.read_exact(&mut ciphertext)
            .map_err(|_| ProjectionError::Persistence)?;
        let mut tail = [0; 1];
        if file
            .read(&mut tail)
            .map_err(|_| ProjectionError::Persistence)?
            != 0
        {
            return Err(ProjectionError::Persistence);
        }
        Ok(Some(SealedCell { writer, ciphertext }))
    }
    pub(super) fn stage(
        &self,
        ciphertext: &[u8],
        writer: WriterEpoch,
    ) -> Result<AtomicPublication, ProjectionError> {
        if ciphertext.len() > self.maximum {
            return Err(ProjectionError::Capacity);
        }
        let mut bytes = Vec::with_capacity(16 + ciphertext.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&writer.get().to_be_bytes());
        bytes.extend_from_slice(ciphertext);
        AtomicPublication::stage_private(&self.path, &bytes)
            .map_err(|_| ProjectionError::Persistence)
    }
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub(super) fn confirm(&self, file: &File) -> Result<(), ProjectionError> {
        require_private_regular(file).map_err(|_| ProjectionError::Persistence)?;
        file.sync_all()
            .and_then(|()| self.parent.sync_all())
            .map_err(|_| ProjectionError::Persistence)
    }
}
fn coordinate(
    chat: ChatCoordinate,
    writer: WriterEpoch,
) -> Result<JournalCoordinate, ProjectionError> {
    // Local view header uses the actual ChatId as its map key, NOT an EventId.
    let id = chat.chat_id();
    JournalCoordinate::new(chat, JournalClass::Header, id, 0, 0, writer.get())
        .map_err(|_| ProjectionError::InvalidRecord)
}
pub(super) fn seal(
    key: &ChatKey,
    chat: ChatCoordinate,
    marker: &CacheMarker,
    records: &[CachedPublication],
) -> Result<Vec<u8>, ProjectionError> {
    if records.len() > MAX_REPLAY_EVENTS {
        return Err(ProjectionError::Capacity);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.extend_from_slice(PLAIN);
    bytes.extend_from_slice(&marker.catalog_revision.to_be_bytes());
    bytes.extend_from_slice(marker.catalog_root.as_bytes());
    bytes.extend_from_slice(&marker.control.get().to_be_bytes());
    bytes.push(u8::from(marker.deleted));
    bytes.extend_from_slice(&marker.known.sequence().get().to_be_bytes());
    match marker.known.head() {
        Some(head) => {
            bytes.push(1);
            bytes.extend_from_slice(head.as_bytes());
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(&marker.delivered.to_be_bytes());
    bytes.extend_from_slice(&(records.len() as u16).to_be_bytes());
    for record in records {
        bytes.extend_from_slice(&record.metadata.id());
        bytes.extend_from_slice(&record.delivery.to_be_bytes());
        field(
            &mut bytes,
            &record
                .metadata
                .binding()
                .canonical_bytes()
                .map_err(|_| ProjectionError::InvalidRecord)?,
        )?;
        field(&mut bytes, &record.ciphertext)?;
    }
    key.seal_journal(&coordinate(chat, marker.writer)?, &bytes)
        .map_err(|_| ProjectionError::Persistence)
}
pub(super) fn open_marker(
    key: &ChatKey,
    chat: ChatCoordinate,
    upper: WriterEpoch,
    cell: &SealedCell,
) -> Result<CacheMarker, ProjectionError> {
    let writer = WriterEpoch::new(cell.writer).map_err(|_| ProjectionError::InvalidRecord)?;
    if writer > upper {
        return Err(ProjectionError::Rollback);
    }
    let bytes = key
        .open_journal(&coordinate(chat.clone(), writer)?, &cell.ciphertext)
        .map_err(|_| ProjectionError::InvalidRecord)?;
    let mut input = Input(&bytes);
    if input.take(8)? != PLAIN {
        return Err(ProjectionError::InvalidRecord);
    }
    let catalog_revision = u64::from_be_bytes(input.array()?);
    let catalog_root = Fingerprint::from_bytes(input.array()?);
    let control = ChatControlRevision::from_u64(u64::from_be_bytes(input.array()?));
    let deleted = match input.array::<1>()? {
        [0] => false,
        [1] => true,
        _ => return Err(ProjectionError::InvalidRecord),
    };
    let sequence = EventSequence::from_u64(u64::from_be_bytes(input.array()?));
    let known = match input.array::<1>()? {
        [0] if sequence.get() == 0 => ProjectionCursor::origin(),
        [1] if sequence.get() > 0 => {
            ProjectionCursor::at(sequence, Fingerprint::from_bytes(input.array()?))
        }
        _ => return Err(ProjectionError::InvalidRecord),
    };
    let delivered = u64::from_be_bytes(input.array()?);
    let count = usize::from(u16::from_be_bytes(input.array()?));
    if count > MAX_REPLAY_EVENTS || catalog_revision == 0 {
        return Err(ProjectionError::InvalidRecord);
    }
    validate_records(&mut input, count, &chat, upper, known, delivered)?;
    if !input.0.is_empty() {
        return Err(ProjectionError::InvalidRecord);
    }
    Ok(CacheMarker {
        writer,
        known,
        control,
        deleted,
        catalog_revision,
        catalog_root,
        delivered,
    })
}

fn validate_records(
    input: &mut Input<'_>,
    count: usize,
    chat: &ChatCoordinate,
    upper: WriterEpoch,
    known: ProjectionCursor,
    delivered: u64,
) -> Result<(), ProjectionError> {
    let mut last_delivery = 0;
    let mut last_sequence = 0;
    for _ in 0..count {
        let _publication = input.array::<16>()?;
        let delivery = u64::from_be_bytes(input.array()?);
        let binding = RelayBinding::from_canonical_bytes(input.field(MAX_RELAY_BINDING_BYTES)?)
            .map_err(|_| ProjectionError::InvalidRecord)?;
        let ciphertext = input.field(nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES)?;
        let context = binding
            .record_context()
            .ok_or(ProjectionError::InvalidRecord)?;
        let sequence = context.position().sequence;
        if context.chat() != chat
            || context.position().writer_epoch > upper.get()
            || sequence == 0
            || sequence > known.sequence().get()
            || delivery <= last_delivery
            || delivery > delivered
            || (last_sequence != 0 && sequence != last_sequence + 1)
        {
            return Err(ProjectionError::InvalidRecord);
        }
        if sequence == known.sequence().get()
            && known.head() != Some(Fingerprint::from_bytes(Sha256::digest(ciphertext).into()))
        {
            return Err(ProjectionError::InvalidRecord);
        }
        last_delivery = delivery;
        last_sequence = sequence;
    }

    Ok(())
}
