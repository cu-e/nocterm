//! Selected borrowed serialization into the shared private publication stage.
pub(crate) mod managed;
#[cfg(test)]
mod managed_tests;
pub(crate) mod managed_writer;
pub(crate) mod native_paths;
pub(crate) mod native_stage;
use nocterm_ai::history::HistoryCodecError;
#[cfg(test)]
use nocterm_ai::history::{EncodingDecision, MAX_FILE_BYTES};
#[cfg(test)]
use nocterm_core::fs::{AtomicPublication, StageError};
#[cfg(test)]
use serde::Serialize;
use std::io;
#[cfg(test)]
use std::{io::Write, path::Path};

#[derive(Debug, thiserror::Error)]
pub(super) enum ProducerError {
    #[error(transparent)]
    Codec(#[from] HistoryCodecError),
    #[error("snapshot write failed: {0}")]
    Io(#[source] io::Error),
}

/// A bounded writer owns only a borrow, actual count, and the first typed failure.
#[cfg(test)]
struct CountedWriter<'a> {
    writer: &'a mut dyn Write,
    bytes: usize,
    failure: Option<ProducerError>,
}
#[cfg(test)]
impl Write for CountedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::Error::from(io::ErrorKind::Other));
        }
        let next = self.bytes.checked_add(bytes.len());
        let refusal = match next {
            None => Some(HistoryCodecError::ArithmeticOverflow),
            Some(next) if next > MAX_FILE_BYTES as usize => Some(HistoryCodecError::EncodedLimit),
            _ => None,
        };
        if let Some(error) = refusal {
            self.failure = Some(ProducerError::Codec(error));
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        match self.writer.write(bytes) {
            Ok(0) if !bytes.is_empty() => {
                self.failure = Some(ProducerError::Io(io::ErrorKind::WriteZero.into()));
                Err(io::Error::from(io::ErrorKind::WriteZero))
            }
            Ok(count) => {
                self.bytes += count;
                Ok(count)
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => {
                self.failure = Some(ProducerError::Io(error));
                Err(io::Error::from(io::ErrorKind::Other))
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
pub(super) fn stage(
    path: &Path,
    value: &impl Serialize,
    decision: EncodingDecision,
) -> Result<AtomicPublication, StageError<ProducerError>> {
    AtomicPublication::stage_private_with(path, |writer| produce(writer, value, decision))
}
#[cfg(test)]
fn produce(
    writer: &mut dyn Write,
    value: &impl Serialize,
    decision: EncodingDecision,
) -> Result<(), ProducerError> {
    let mut counter = CountedWriter {
        writer,
        bytes: 0,
        failure: None,
    };
    let result = serde_json::to_writer(&mut counter, value);
    if let Some(error) = counter.failure {
        return Err(error);
    }
    result.map_err(HistoryCodecError::from)?;
    if counter.bytes != decision.size.encoded_bytes {
        return Err(HistoryCodecError::CandidateMismatch.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::history::{HistoryEncoding, measure_json};
    use serde::{
        Serializer,
        ser::{Error as _, SerializeSeq},
    };

    struct Rejected;
    impl Serialize for Rejected {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element("written prefix")?;
            Err(S::Error::custom("semantic producer refusal"))
        }
    }
    #[test]
    fn rejected_serialization_discards_partial_private_stage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot");
        std::fs::write(&path, b"old").unwrap();
        let decision = EncodingDecision {
            encoding: HistoryEncoding::Version2,
            size: Default::default(),
        };
        assert!(matches!(
            stage(&path, &Rejected, decision),
            Err(StageError::Producer(ProducerError::Codec(
                HistoryCodecError::Json(_)
            )))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    #[test]
    fn final_count_refusal_preserves_old_file_and_success_matches_exact_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot");
        std::fs::write(&path, b"old").unwrap();
        let value = serde_json::json!({"quote\"\n界":[null,{"nested":true}],"literal":"a\\b"});
        let decision = EncodingDecision {
            encoding: HistoryEncoding::Version2,
            size: measure_json(&value).unwrap(),
        };
        let mut rejected = decision;
        rejected.size.encoded_bytes += 1;
        assert!(matches!(
            stage(&path, &value, rejected),
            Err(StageError::Producer(ProducerError::Codec(
                HistoryCodecError::CandidateMismatch
            )))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        stage(&path, &value, decision).unwrap().publish().unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
        assert_eq!(
            nocterm_ai::history::structural_bytes(&bytes).unwrap(),
            decision.size.structural_bytes
        );
    }
}
