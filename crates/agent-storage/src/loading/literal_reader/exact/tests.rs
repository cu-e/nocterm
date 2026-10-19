//! Actual held-span text planning, grammar and current replay capacity behavior.
use super::*;
use crate::loading::{held_file::HeldReadError, json_frames::RootFrames};
use std::{fs, io::Cursor};

#[test]
fn measured_scalar_uses_decoded_unicode_length_and_original_source_identity() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.json");
    let source = br#"{"x":"\u0061\u00e9\ud83d\ude00"}"#;
    fs::write(&path, source).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap() else {
                panic!()
            };
            frames.next_member().unwrap().unwrap()
        })
        .unwrap();
    let value = decode_authenticated_string(&plan, &mut held, |field| field.native_span(), &ledger)
        .unwrap()
        .unwrap();
    assert_eq!(value.as_owned().as_value().as_str(), Some("aé😀"));
    assert!(value.as_owned().receipt().matches(value.as_owned()));
    assert_eq!(fs::read(&path).unwrap(), source);
    drop(value);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn scalar_measurement_keeps_stock_unicode_and_trailing_token_rejections() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    for source in [
        br#""\ud800""#.as_slice(),
        br#""a" null"#,
        b"null",
        br#""truncated"#,
    ] {
        assert!(measure(&mut Cursor::new(source), &ledger).is_err());
    }
    assert_eq!(
        measure(&mut Cursor::new(br#""\u0061\u0062""#), &ledger).unwrap(),
        2
    );
    assert_eq!(measure(&mut Cursor::new(br#"  ""  "#), &ledger).unwrap(), 0);
}
#[test]
fn changed_native_scalar_is_never_adopted_even_if_new_length_matches() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.json");
    fs::write(&path, br#"{"x":"aaa"}"#).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap() else {
                panic!()
            };
            frames.next_member().unwrap().unwrap()
        })
        .unwrap();
    let measured = plan
        .replay(
            &mut held,
            |field| field.native_span(),
            |reader| measure(reader, &ledger),
        )
        .unwrap()
        .unwrap();
    fs::write(&path, br#"{"x":"bbb"}"#).unwrap();
    let result = plan.replay(
        &mut held,
        |field| field.native_span(),
        |reader| build(reader, measured, &ledger),
    );
    assert!(matches!(result, Err(HeldReadError::ChangedDocument)));
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn changed_escape_layout_is_stopped_before_measured_backing_can_grow() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.json");
    let original = br#"{"x":"\u0061"}"#;
    let changed = br#"{"x":"abcdef"}"#;
    assert_eq!(original.len(), changed.len());
    fs::write(&path, original).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap() else {
                panic!()
            };
            frames.next_member().unwrap().unwrap()
        })
        .unwrap();
    let measured = plan
        .replay(
            &mut held,
            |field| field.native_span(),
            |reader| measure(reader, &ledger),
        )
        .unwrap()
        .unwrap();
    assert_eq!(measured, 1);
    fs::write(&path, changed).unwrap();
    let stopped_before_growth = std::cell::Cell::new(false);
    let result = plan.replay(
        &mut held,
        |field| field.native_span(),
        |reader| {
            let result = build(reader, measured, &ledger);
            stopped_before_growth.set(matches!(
                result,
                Err(LiteralReadError::Build(JsonDecodeError::TextLength {
                    expected: 1,
                    actual: 2
                }))
            ));
            result
        },
    );
    assert!(stopped_before_growth.get());
    assert!(matches!(result, Err(HeldReadError::ChangedDocument)));
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
