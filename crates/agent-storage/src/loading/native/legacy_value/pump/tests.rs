//! Real cursor and credit lifetime behavior; no parser output caches are required.
use super::*;
use nocterm_ai::{history::MAX_RESIDENT_BYTES, model::retention::RetentionAuthority};
use std::io::Cursor;
#[test]
fn streaming_layer_resumes_parent_after_unicode_and_escaped_quotes() {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    for value in ["é中", "𝄞", "line\n\"quoted\"", "\\", ""] {
        let encoded = format!("{},null", serde_json::to_string(value).unwrap());
        let mut pump = BytePump::new(Cursor::new(encoded.as_bytes()), &ledger).unwrap();
        let mut quote = [0];
        pump.read_exact(&mut quote).unwrap();
        assert_eq!(quote, [b'"']);
        pump.enter().unwrap();
        let mut decoded = String::new();
        pump.read_to_string(&mut decoded).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(pump.leave().unwrap(), encoded.len() - 5);
        let mut suffix = String::new();
        pump.read_to_string(&mut suffix).unwrap();
        assert_eq!(suffix, ",null");
    }
}
#[test]
fn lexical_failure_and_frame_refusal_release_actual_scratch_after_drop() {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, 32 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    {
        let mut pump = BytePump::new(Cursor::new(br#""\uD834x""#), &ledger).unwrap();
        let mut byte = [0];
        pump.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'"']);
        pump.enter().unwrap();
        assert!(pump.read(&mut byte).is_err());
        assert!(pump.take_failure().is_some());
    }
    assert_eq!(ledger.live_requested_bytes(), baseline);
    {
        let mut pump = BytePump::new(Cursor::new([]), &ledger).unwrap();
        while pump.enter().is_ok() {}
    }
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
