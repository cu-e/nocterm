use super::*;
use nocterm_ai::history::{HistoryEncoding, measure_json};
use std::error::Error;

impl ManagedWire for serde_json::Value {}

struct NativeFailure {
    file: tempfile::NamedTempFile,
    writes: usize,
    original: Option<*const (dyn Error + 'static)>,
}
impl Write for NativeFailure {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        let error = self.file.write(bytes).unwrap_err();
        self.original = error.source().map(|source| source as *const _);
        Err(error)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("JSON producer never flushes");
    }
}
#[test]
fn closed_managed_wire_stops_native_io_and_preserves_first_original_context() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("read-only-handle");
    std::fs::write(&path, b"old").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let temp = tempfile::TempPath::from_prepared_path(path.clone().into_boxed_path());
    let mut native = NativeFailure {
        file: tempfile::NamedTempFile::from_parts(file, temp),
        writes: 0,
        original: None,
    };
    let value = serde_json::json!({"first":"literal", "later":["a".repeat(1000), 2, true]});
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size: measure_json(&value).unwrap(),
    };
    let error = produce(&mut native, &value, decision).unwrap_err();
    let ProducerError::Io(error) = error else {
        panic!("original native IO wins");
    };
    assert_eq!(native.writes, 1);
    assert_eq!(
        error.source().map(|source| source as *const _),
        native.original
    );
    assert!(error.to_string().contains(" at path "));
    assert!(error.to_string().contains("read-only-handle"));
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
}
#[test]
fn successful_closed_wire_preserves_exact_bytes_and_rejects_wrong_final_count() {
    let value = serde_json::json!({"quote\"\n界":[null, {"nested": true}]});
    let decision = EncodingDecision {
        encoding: HistoryEncoding::Version2,
        size: measure_json(&value).unwrap(),
    };
    let mut bytes = Vec::new();
    produce(&mut bytes, &value, decision).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
    let mut wrong = decision;
    wrong.size.encoded_bytes += 1;
    assert!(matches!(
        produce(&mut Vec::new(), &value, wrong),
        Err(ProducerError::Codec(HistoryCodecError::CandidateMismatch))
    ));
}
