//! Native original-held-file typed row construction, without JSON tree bridge.
use super::*;
use crate::loading::v2_plan::Scope;
use nocterm_ai::thread::Entry;
use std::fs;
fn row(bytes: &[u8], ledger: &DecodeLedger) -> Result<DecodedRow, RowReadError> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("row.json");
    fs::write(&path, bytes).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Entry))
        .unwrap()
        .transpose()
        .unwrap();
    let row = read_text(&plan, &mut held, ledger)?;
    assert_eq!(fs::read(path).unwrap(), bytes);
    Ok(row)
}
#[test]
fn direct_text_rows_match_actual_native_entry_wire_for_early_late_and_escaped_tags() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for bytes in [
        r#"{"kind":"agent","value":" literal 日本 "}"#.as_bytes(),
        br#"{"value":"thought\n\uD834\uDD1E","kind":"thought"}"#,
        br#"{"unknown":{"raw":"ignored"},"kind":"\u0061gent","value":"kept"}"#,
    ] {
        let expected: Entry = crate::history_fixtures::decoded(&authority, bytes).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let actual = row(bytes, &ledger).unwrap();
        assert_eq!(actual.as_entry(), &expected);
        assert!(ledger.live_requested_bytes() > baseline);
        drop(actual);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
#[test]
fn direct_text_row_keeps_raw_marker_spelling_inside_ordinary_string() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let bytes = br#"{"kind":"agent","value":"$serde_json::private::RawValue"}"#;
    let actual = row(bytes, &DecodeLedger::new(&authority, 4096).unwrap()).unwrap();
    assert_eq!(
        actual.as_entry(),
        &Entry::Agent("$serde_json::private::RawValue".into())
    );
}
#[test]
fn original_adjacent_enum_sequence_grammar_is_preserved_by_direct_text_route() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let bytes = br#"["thought","literal"]"#;
    let expected: Entry = crate::history_fixtures::decoded(&authority, bytes).unwrap();
    let actual = row(bytes, &DecodeLedger::new(&authority, 4096).unwrap()).unwrap();
    assert_eq!(actual.as_entry(), &expected);
}
#[test]
fn unknown_and_composite_tags_do_not_silently_become_text_rows() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    assert!(matches!(
        row(
            br#"{"kind":"future","value":"x"}"#,
            &DecodeLedger::new(&authority, 4096).unwrap()
        ),
        Err(RowReadError::UnknownKind)
    ));
    assert!(matches!(
        row(
            br#"{"kind":"user","value":[]}"#,
            &DecodeLedger::new(&authority, 4096).unwrap()
        ),
        Err(RowReadError::Composite(SavedRowKind::User))
    ));
}
#[test]
fn changed_text_row_replay_cannot_adopt_new_bytes_or_leak_reservations() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original = br#"{"kind":"agent","value":"original"}"#;
    let changed = br#"{"kind":"agent","value":"replaced"}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("row.json");
    fs::write(&path, original).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Entry))
        .unwrap()
        .transpose()
        .unwrap();
    fs::write(&path, changed).unwrap();
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let result = read_text(&plan, &mut held, &ledger);
    fs::write(&path, original).unwrap();
    assert!(matches!(
        result,
        Err(RowReadError::Held(HeldReadError::ChangedDocument))
    ));
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
