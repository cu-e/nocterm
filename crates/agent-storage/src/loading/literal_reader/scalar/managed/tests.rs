//! Stock scalar acceptance, original lexical errors and prospective admission.
use super::*;
use nocterm_ai::model::retention::RetentionAuthority;
use std::{error::Error as _, io::Cursor};

#[test]
fn managed_scalar_keeps_stock_lexical_cause_positions_and_last_clone_credit() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    for encoded in ["-", "00", "1e+", "1.", "1e999999", "tru", "nulx"] {
        let expected = serde_json::from_str::<serde_json::Value>(encoded).unwrap_err();
        let mut input = Input::new(Cursor::new(encoded));
        let error = match read(&mut input, &ledger) {
            Err(error) => error,
            Ok(_) => panic!("stock malformed scalar"),
        };
        let LiteralReadError::ManagedJson(failure) = error else {
            panic!("original guarded lexical cause")
        };
        let original = failure
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .unwrap();
        assert_eq!(original.classify(), expected.classify(), "{encoded}");
        assert_eq!(
            (original.line(), original.column()),
            (expected.line(), expected.column()),
            "{encoded}"
        );
        assert_eq!(original.to_string(), expected.to_string(), "{encoded}");
        let clone = failure.clone();
        assert!(std::ptr::eq(
            failure.source().unwrap(),
            clone.source().unwrap()
        ));
        let live = ledger.live_requested_bytes();
        drop(failure);
        assert_eq!(ledger.live_requested_bytes(), live);
        drop(clone);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
#[test]
fn managed_scalar_admission_refuses_before_original_parser_reads_source() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let probe = DecodeLedger::new(&authority, 65536).unwrap();
    let fixed = probe.live_requested_bytes();
    drop(probe);
    let ledger = DecodeLedger::new(&authority, fixed).unwrap();
    let mut input = Input::new(Cursor::new("1e999999"));
    assert!(matches!(
        read(&mut input, &ledger),
        Err(LiteralReadError::Admission(_))
    ));
    assert_eq!(input.offset(), 0);
    assert_eq!(ledger.live_requested_bytes(), fixed);
}
