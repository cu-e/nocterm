//! Closed managed serialization retains the first native failure without JSON IO boxes.
use super::ProducerError;
use nocterm_ai::history::{EncodingDecision, HistoryCodecError, MAX_FILE_BYTES};
use serde::Serialize;
use std::io::{self, Write};

/// Only the finite borrowed managed serializers implement this private port.
/// They invoke no external Serialize callbacks or observable producer side effects.
pub(crate) trait ManagedWire: Serialize {}
impl ManagedWire for nocterm_ai::history::physical::SnapshotWire<'_> {}
impl ManagedWire for nocterm_ai::history::NativeDocumentWire<'_> {}
impl ManagedWire for nocterm_ai::history::ChatSummaryWire<'_> {}

struct ManagedWriter<'a> {
    writer: &'a mut dyn Write,
    bytes: usize,
    failure: Option<ProducerError>,
}
impl Write for ManagedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            // Drain only the closed, pure serializer. Never touch native IO again.
            return Ok(bytes.len());
        }
        match self.bytes.checked_add(bytes.len()) {
            None => {
                self.failure = Some(ProducerError::Codec(HistoryCodecError::ArithmeticOverflow))
            }
            Some(next) if next > MAX_FILE_BYTES as usize => {
                self.failure = Some(ProducerError::Codec(HistoryCodecError::EncodedLimit));
            }
            _ => match self.writer.write(bytes) {
                Ok(0) if !bytes.is_empty() => {
                    self.failure = Some(ProducerError::Io(io::ErrorKind::WriteZero.into()));
                }
                Ok(count) => {
                    self.bytes += count;
                    return Ok(count);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => self.failure = Some(ProducerError::Io(error)),
            },
        }
        // The first failure is reported by finish; it can never reach publication.
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.failure.is_some() {
            return Ok(());
        }
        match self.writer.flush() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.failure = Some(ProducerError::Io(error));
                Ok(())
            }
        }
    }
}
pub(super) fn produce(
    writer: &mut dyn Write,
    value: &impl ManagedWire,
    decision: EncodingDecision,
) -> Result<(), ProducerError> {
    let mut writer = ManagedWriter {
        writer,
        bytes: 0,
        failure: None,
    };
    let encoded = serde_json::to_writer(&mut writer, value);
    if let Some(failure) = writer.failure {
        return Err(failure);
    }
    encoded.map_err(HistoryCodecError::from)?;
    if writer.bytes != decision.size.encoded_bytes {
        return Err(HistoryCodecError::CandidateMismatch.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
