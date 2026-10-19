//! Actual held-file scope plans and original typed v2 grammar boundaries.
use super::*;
use crate::loading::{
    held_file::{HeldFile, HeldReadError},
    literal_reader::decode_retained,
};
use nocterm_ai::{
    history::{ChatMetadata, ComposerData, HistoryCodecError, ResumeDescriptor, TranscriptData},
    model::json::deferred::DecodeLedger,
};
use std::{fs, io::Cursor};
fn scan(bytes: &[u8], scope: Scope) -> Result<V2Plan, PlanError> {
    V2Plan::scan(Cursor::new(bytes), 0, scope)
}
#[test]
fn envelope_rejects_unknown_duplicate_missing_and_trailing_fields() {
    assert!(
        scan(
            br#"{"format":"nocterm.chat","version":2,"document":{}}"#,
            Scope::Envelope
        )
        .is_ok()
    );
    for bytes in [
        br#"{"format":"nocterm.chat","version":2,"document":{},"extra":0}"#.as_slice(),
        br#"{"format":"nocterm.chat","version":2,"document":{},"version":2}"#,
        br#"{"format":"nocterm.chat","version":2}"#,
        br#"{"format":"nocterm.chat","version":2,"document":{}}false"#,
        br#"["nocterm.chat",2,{}]"#,
    ] {
        assert!(scan(bytes, Scope::Envelope).is_err());
    }
}
#[test]
fn typed_nested_default_and_optional_presence_match_original_map_grammar() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let metadata = br#"{"agent_id":"a","pinned":false,"updated":0,"unknown":7}"#;
    assert!(serde_json::from_slice::<ChatMetadata>(metadata).is_ok());
    assert!(scan(metadata, Scope::Metadata).is_ok());
    let resume = br#"{"config":{},"pending_history":false}"#;
    assert!(serde_json::from_slice::<ResumeDescriptor>(resume).is_ok());
    assert!(scan(resume, Scope::Resume).is_ok());
    for bytes in [
        br#"{"agent_id":"a","updated":0}"#.as_slice(),
        br#"{"agent_id":"a","pinned":false}"#,
    ] {
        assert!(serde_json::from_slice::<ChatMetadata>(bytes).is_err());
        assert!(scan(bytes, Scope::Metadata).is_err());
    }
    assert!(crate::history_fixtures::decoded::<ComposerData>(&authority, b"{}").is_ok());
    assert!(scan(b"{}", Scope::Composer).is_ok());
    assert!(
        crate::history_fixtures::decoded::<TranscriptData>(&authority, br#"{"entries":[]}"#)
            .is_ok()
    );
    assert!(scan(br#"{"entries":[]}"#, Scope::Transcript).is_ok());
}
#[test]
fn nested_positional_default_tail_and_missing_optionals_match_serde_sequences() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for bytes in [br#"[null]"#.as_slice(), br#"[null,[],[]]"#] {
        assert!(crate::history_fixtures::decoded::<ComposerData>(&authority, bytes).is_ok());
        assert!(scan(bytes, Scope::Composer).is_ok());
    }
    assert!(crate::history_fixtures::decoded::<ComposerData>(&authority, b"[]").is_err());
    assert!(scan(b"[]", Scope::Composer).is_err());
    for bytes in [b"[[]]".as_slice(), b"[[],[]]"] {
        assert!(crate::history_fixtures::decoded::<TranscriptData>(&authority, bytes).is_ok());
        assert!(scan(bytes, Scope::Transcript).is_ok());
    }
    let metadata = br#"["a",null,null,false,0,null]"#;
    assert!(serde_json::from_slice::<ChatMetadata>(metadata).is_ok());
    assert!(scan(metadata, Scope::Metadata).is_ok());
    let short = br#"["a",null,null,false,0]"#;
    assert!(serde_json::from_slice::<ChatMetadata>(short).is_err());
    assert!(scan(short, Scope::Metadata).is_err());
}
#[test]
fn deep_literal_span_framing_does_not_impose_global_128_validation_policy() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let source = format!(
        r#"{{"format":"nocterm.chat","version":2,"document":{}0{}}}"#,
        "[".repeat(200),
        "]".repeat(200)
    );
    let plan = scan(source.as_bytes(), Scope::Envelope).unwrap();
    let span = plan.field(Key::Document).unwrap().value();
    let bytes = &source.as_bytes()[span.start()..span.start() + span.len()];
    assert!(
        decode_retained(
            Cursor::new(bytes),
            &DecodeLedger::new(&authority, 1024 * 1024).unwrap()
        )
        .is_ok()
    );
    // Shape framing is deliberately weaker than semantic parsing.
    let malformed = br#"{"format":"nocterm.chat","version":2,"document":[}}"#;
    let plan = scan(malformed, Scope::Envelope).unwrap();
    let span = plan.field(Key::Document).unwrap().value();
    assert!(
        decode_retained(
            Cursor::new(&malformed[span.start()..span.start() + span.len()]),
            &DecodeLedger::new(&authority, 1024 * 1024).unwrap()
        )
        .is_err()
    );
}
#[test]
fn authenticated_nested_document_scope_preserves_exact_original_field_spans() {
    let source=br#"{"version":2,"document":{"id":"original","metadata":{},"resume":{},"transcript":{"entries":[]},"composer":{}},"format":"nocterm.chat"}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.json");
    fs::write(&path, source).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    let root = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Envelope))
        .unwrap()
        .transpose()
        .unwrap();
    header::check(&root, &mut held).unwrap();
    let start = root.payload().field(Key::Document).unwrap().value().start();
    let document = root
        .derive(
            &mut held,
            |plan| plan.field(Key::Document).unwrap().native_span(),
            |reader| V2Plan::scan(reader, start, Scope::Document),
        )
        .unwrap()
        .transpose()
        .unwrap();
    let id = document
        .replay(
            &mut held,
            |plan| plan.field(Key::Id).unwrap().native_span(),
            |reader| serde_json::from_reader::<_, String>(reader),
        )
        .unwrap()
        .unwrap();
    assert_eq!(id, "original");
    assert_eq!(fs::read(path).unwrap(), source);
}
#[test]
fn nested_scan_mutation_then_restore_cannot_fabricate_authenticated_plan() {
    let original=br#"{"version":2,"format":"nocterm.chat","document":{"id":"original","metadata":{},"resume":{},"transcript":{},"composer":{}}}"#;
    let changed = String::from_utf8(original.to_vec())
        .unwrap()
        .replace("original", "replaced");
    assert_eq!(original.len(), changed.len());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.json");
    fs::write(&path, original).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    let root = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Envelope))
        .unwrap()
        .transpose()
        .unwrap();
    fs::write(&path, &changed).unwrap();
    let start = root.payload().field(Key::Document).unwrap().value().start();
    let result = root.derive(
        &mut held,
        |plan| plan.field(Key::Document).unwrap().native_span(),
        |reader| {
            let plan = V2Plan::scan(reader, start, Scope::Document);
            fs::write(&path, original).unwrap();
            plan
        },
    );
    assert!(matches!(result, Err(HeldReadError::ChangedDocument)));
}
#[test]
fn future_version_header_keeps_typed_error_without_document_materialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chat.json");
    fs::write(
        &path,
        br#"{"format":"nocterm.chat","version":999,"document":{}}"#,
    )
    .unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight().unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Envelope))
        .unwrap()
        .transpose()
        .unwrap();
    assert!(matches!(
        header::check(&plan, &mut held),
        Err(header::HeaderError::Codec(
            HistoryCodecError::UnsupportedVersion(999)
        ))
    ));
}
