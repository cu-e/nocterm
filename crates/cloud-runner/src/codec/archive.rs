//! Bounded native recovery snapshots; these bytes are never public hub layouts.
use super::{Checkpoint, identity::Stream, record::Record};
use nocterm_cloud_model::{
    authority::StreamBinding, engine::EngineCheckpoint, identity::*, journal::*,
    payload::OpaqueBytes,
};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"NCRA\x02";
pub(crate) const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_RECORDS: usize = 4096;
const MAX_STREAM_BYTES: usize = 2048;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ActionAnchor {
    record: u64,
    revision: u64,
    step: StepId,
}
pub(crate) struct NativeRunArchive {
    stream: StreamBinding,
    at: JournalPosition,
    anchor: Option<ActionAnchor>,
    checkpoint: Option<EngineCheckpoint>,
    records: Vec<StoredRecord>,
}
impl NativeRunArchive {
    pub(crate) fn new(
        stream: StreamBinding,
        at: JournalPosition,
        records: Vec<StoredRecord>,
        checkpoint: Option<EngineCheckpoint>,
    ) -> Result<Self, JournalError> {
        if records.is_empty()
            || records.len() > MAX_RECORDS
            || records.len() as u64 != at.records().get()
        {
            return Err(JournalError::Capacity);
        }
        let mut previous = JournalSequence::default();
        let mut revision = StreamRevision::default();
        let mut control = 0u64;
        let mut changed = false;
        let mut anchor = None;
        for stored in &records {
            if stored.stream() != &stream
                || stored.sequence() != previous.advance(1)?
                || stored.revision() < revision
                || stored.revision() > at.revision()
            {
                return Err(JournalError::Corrupt);
            }
            if stored.revision() != revision {
                control += u64::from(changed);
                changed = false;
            }
            changed |= stored.record().changes_control();
            previous = stored.sequence();
            revision = stored.revision();
            if let JournalRecordKind::ModelAction { step } = stored.record().kind() {
                anchor = Some(ActionAnchor {
                    record: previous.get(),
                    revision: revision.get(),
                    step: *step,
                });
            }
        }
        control += u64::from(changed);
        if control != at.control().get() {
            return Err(JournalError::Corrupt);
        }
        match (anchor, checkpoint.as_ref()) {
            (Some(anchor), Some(checkpoint))
                if checkpoint.journal_revision().get() == anchor.revision
                    && records.iter().any(|record| {
                        record.sequence().get() == anchor.record
                            && record.record().context().run() == Some(checkpoint.run())
                    }) => {}
            (None, None) => {}
            _ => return Err(JournalError::Conflict),
        }
        Ok(Self {
            stream,
            at,
            anchor,
            checkpoint,
            records,
        })
    }
    pub(crate) fn into_records(self) -> Vec<StoredRecord> {
        self.records
    }
    pub(crate) fn encode(&self) -> Result<OpaqueBytes, JournalError> {
        let binding = postcard::to_allocvec(&Stream::from(&self.stream))
            .map_err(|_| JournalError::Corrupt)?;
        if binding.len() > MAX_STREAM_BYTES {
            return Err(JournalError::Capacity);
        }
        let mut output = Vec::with_capacity(MAX_ARCHIVE_BYTES);
        output.extend_from_slice(MAGIC);
        push(&mut output, &(binding.len() as u32).to_le_bytes())?;
        push(&mut output, &binding)?;
        for value in [
            self.at.revision().get(),
            self.at.control().get(),
            self.at.records().get(),
            self.at.events().get(),
        ] {
            push(&mut output, &value.to_le_bytes())?;
        }
        match (self.anchor, self.checkpoint.as_ref()) {
            (Some(anchor), Some(checkpoint)) => {
                push(&mut output, &[1])?;
                push(&mut output, &anchor.record.to_le_bytes())?;
                push(&mut output, &anchor.revision.to_le_bytes())?;
                push(&mut output, anchor.step.as_bytes())?;
                let bytes = Zeroizing::new(
                    postcard::to_allocvec(&Checkpoint::from(checkpoint))
                        .map_err(|_| JournalError::Corrupt)?,
                );
                push(&mut output, &(bytes.len() as u32).to_le_bytes())?;
                push(&mut output, &bytes)?;
            }
            (None, None) => push(&mut output, &[0])?,
            _ => return Err(JournalError::Corrupt),
        }
        push(&mut output, &(self.records.len() as u32).to_le_bytes())?;
        for stored in &self.records {
            if stored.record().payload().len() > MAX_ARCHIVE_BYTES {
                return Err(JournalError::Capacity);
            }
            let bytes = Zeroizing::new(
                postcard::to_allocvec(&Record::from(stored.record()))
                    .map_err(|_| JournalError::Corrupt)?,
            );
            push(&mut output, &stored.sequence().get().to_le_bytes())?;
            push(&mut output, &stored.revision().get().to_le_bytes())?;
            push(&mut output, &(bytes.len() as u32).to_le_bytes())?;
            push(&mut output, &bytes)?;
        }
        Ok(OpaqueBytes::new(output)?)
    }
    pub(crate) fn decode(snapshot: &JournalSnapshot) -> Result<Self, JournalError> {
        let bytes = snapshot.payload().as_bytes();
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err(JournalError::Capacity);
        }
        let mut decoder = Decoder { bytes, offset: 0 };
        if decoder.take(MAGIC.len())? != MAGIC {
            return Err(JournalError::UnsupportedFormat);
        }
        let stream_len = decoder.u32()? as usize;
        if stream_len > MAX_STREAM_BYTES {
            return Err(JournalError::Capacity);
        }
        let binding = decoder.take(stream_len)?;
        let at = JournalPosition::new(
            StreamRevision::from_u64(decoder.u64()?),
            ChatControlRevision::from_u64(decoder.u64()?),
            JournalSequence::from_u64(decoder.u64()?),
            EventSequence::from_u64(decoder.u64()?),
        );
        let (anchor, checkpoint) = match decoder.take(1)?[0] {
            0 => (None, None),
            1 => {
                let anchor = ActionAnchor {
                    record: decoder.u64()?,
                    revision: decoder.u64()?,
                    step: StepId::from_bytes(
                        decoder
                            .take(16)?
                            .try_into()
                            .map_err(|_| JournalError::Corrupt)?,
                    ),
                };
                let length = decoder.u32()? as usize;
                let (checkpoint, trailing) =
                    postcard::take_from_bytes::<Checkpoint>(decoder.take(length)?)
                        .map_err(|_| JournalError::Corrupt)?;
                if !trailing.is_empty() {
                    return Err(JournalError::Corrupt);
                }
                (Some(anchor), Some(checkpoint.restore()?))
            }
            _ => return Err(JournalError::Corrupt),
        };
        let count = decoder.u32()? as usize;
        if count == 0 || count > MAX_RECORDS {
            return Err(JournalError::Capacity);
        }
        if at != snapshot.at() || at.records().get() != count as u64 {
            return Err(JournalError::Corrupt);
        }
        decoder.preflight(count)?;
        let (stream, trailing) =
            postcard::take_from_bytes::<Stream>(binding).map_err(|_| JournalError::Corrupt)?;
        let stream = stream.restore()?;
        if !trailing.is_empty() || stream != *snapshot.stream() {
            return Err(JournalError::Corrupt);
        }
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            let sequence = JournalSequence::from_u64(decoder.u64()?);
            let revision = StreamRevision::from_u64(decoder.u64()?);
            let length = decoder.u32()? as usize;
            let (record, trailing) = postcard::take_from_bytes::<Record>(decoder.take(length)?)
                .map_err(|_| JournalError::Corrupt)?;
            if !trailing.is_empty() {
                return Err(JournalError::Corrupt);
            }
            records.push(StoredRecord::new(
                stream.clone(),
                sequence,
                revision,
                record.restore()?,
            )?);
        }
        let archive = Self::new(stream, at, records, checkpoint)?;
        if archive.anchor != anchor {
            return Err(JournalError::Corrupt);
        }
        Ok(archive)
    }
}
fn push(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), JournalError> {
    if output.len().saturating_add(bytes.len()) > MAX_ARCHIVE_BYTES {
        return Err(JournalError::Capacity);
    }
    output.extend_from_slice(bytes);
    Ok(())
}
struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Decoder<'a> {
    fn preflight(&mut self, count: usize) -> Result<(), JournalError> {
        let entries = self.offset;
        // Check the whole bounded framing before decoding any native payload.
        for _ in 0..count {
            self.take(16)?;
            let length = self.u32()? as usize;
            if length == 0 {
                return Err(JournalError::Corrupt);
            }
            self.take(length)?;
        }
        if self.offset != self.bytes.len() {
            return Err(JournalError::Corrupt);
        }
        self.offset = entries;
        Ok(())
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], JournalError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(JournalError::Corrupt)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(JournalError::Corrupt)?;
        self.offset = end;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, JournalError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| JournalError::Corrupt)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, JournalError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| JournalError::Corrupt)?,
        ))
    }
}
