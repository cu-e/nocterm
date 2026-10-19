//! Fresh-tree moving behavior, including siblings retained beyond parent drop.
use super::*;
use crate::model::{json::deferred::literal::LiteralBuilder, retention::RetentionAuthority};
fn text(builder: &LiteralBuilder, value: &str) -> super::super::literal::LiteralText {
    let mut text = builder.text().unwrap();
    text.append(value).unwrap();
    text
}
#[test]
fn a_consumed_child_outlives_parent_without_releasing_its_text_credit() {
    let authority = RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 2).unwrap();
    builder.start_object().unwrap();
    builder.key(text(&builder, "first")).unwrap();
    builder.string(text(&builder, "original")).unwrap();
    builder.key(text(&builder, "second")).unwrap();
    builder.string(text(&builder, "sibling")).unwrap();
    builder.end_container().unwrap();
    let mut children = builder.finish_native().unwrap().into_children().unwrap();
    let (first_key, first) = children.next_child().unwrap().unwrap();
    assert_eq!(first_key.as_ref().unwrap().as_str(), "first");
    drop(first_key);
    let first = first.into_text().unwrap();
    assert_eq!(first.as_str(), "original");
    drop(children);
    // This remaining String is genuinely alive; its token cannot retire with
    // the parent table, layout, cleanup workspace or unconsumed sibling.
    assert_eq!(ledger.live_requested_bytes(), baseline + first.capacity());
    drop(first);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn native_opaque_adoption_keeps_deep_metadata_without_compatibility_depth_limit() {
    let authority = RetentionAuthority::new(4 * 1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 4 * 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 1).unwrap();
    builder.start_object().unwrap();
    builder.key(text(&builder, "_meta")).unwrap();
    for _ in 0..256 {
        builder.start_array().unwrap();
    }
    builder.string(text(&builder, "retained")).unwrap();
    for _ in 0..256 {
        builder.end_container().unwrap();
    }
    builder.end_container().unwrap();
    let decoded = builder
        .finish_native()
        .unwrap()
        .into_decoded(&ledger)
        .unwrap();
    let mut value = &decoded.as_owned().as_value()["_meta"];
    for _ in 0..256 {
        value = &value.as_array().unwrap()[0];
    }
    assert_eq!(value.as_str(), Some("retained"));
    drop(decoded);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn native_object_iteration_preserves_first_key_position_and_last_duplicate_value() {
    let authority = RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 2).unwrap();
    builder.start_object().unwrap();
    for (key, value) in [("a", "old"), ("b", "second"), ("a", "replacement")] {
        builder.key(text(&builder, key)).unwrap();
        builder.string(text(&builder, value)).unwrap();
    }
    builder.end_container().unwrap();
    let mut children = builder.finish_native().unwrap().into_children().unwrap();
    for (key, value) in [("a", "replacement"), ("b", "second")] {
        let (actual_key, actual_value) = children.next_child().unwrap().unwrap();
        assert_eq!(actual_key.unwrap().as_str(), key);
        assert_eq!(actual_value.text(), Some(value));
    }
    assert!(children.next_child().unwrap().is_none());
    drop(children);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

fn pair(ledger: &DecodeLedger, key: &str, value: &str) -> (NativeText, NativeNode) {
    let mut builder = LiteralBuilder::with_cleanup_frontier(ledger, 1).unwrap();
    builder.start_object().unwrap();
    builder.key(text(&builder, key)).unwrap();
    builder.string(text(&builder, value)).unwrap();
    builder.end_container().unwrap();
    let mut children = builder.finish_native().unwrap().into_children().unwrap();
    let (key, value) = children.next_child().unwrap().unwrap();
    (key.unwrap(), value)
}
#[test]
fn rebuilt_native_object_moves_distinct_cleanup_roots_and_retires_duplicate_payloads() {
    let authority = RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut object = NativeObject::new(&ledger, 3).unwrap();
    for (key, value) in [("a", "old"), ("b", "second"), ("a", "replacement")] {
        let (key, value) = pair(&ledger, key, value);
        object.push(key, value).unwrap();
    }
    let value = object.finish().unwrap().into_decoded(&ledger).unwrap();
    let object = value.as_owned().as_object().unwrap();
    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(object["a"], "replacement");
    assert_eq!(object["b"], "second");
    drop(value);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn rebuilt_native_object_rejects_foreign_payload_without_losing_original_credit() {
    let authority = RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let other = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let (key, own) = pair(&ledger, "a", "own");
    let (_foreign_key, foreign) = pair(&other, "b", "foreign");
    let baseline = ledger.live_requested_bytes();
    let key_bytes = key.capacity();
    let mut object = NativeObject::new(&ledger, 1).unwrap();
    assert!(matches!(
        object.push(key, foreign),
        Err(JsonDecodeError::ForeignLedger)
    ));
    assert_eq!(own.text(), Some("own"));
    drop(object);
    assert_eq!(ledger.live_requested_bytes(), baseline - key_bytes);
}
