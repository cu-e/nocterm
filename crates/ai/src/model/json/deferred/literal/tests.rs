//! Behavior of iterative literal ownership and actual construction reservations.
use super::*;
fn text(builder: &LiteralBuilder, value: &str) -> LiteralText {
    let mut text = builder.text().unwrap();
    text.append(value).unwrap();
    text
}
#[test]
fn literal_reserved_key_is_retained_as_data_instead_of_sdk_raw_expansion() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    builder.start_object().unwrap();
    builder
        .key(text(&builder, "$serde_json::private::RawValue"))
        .unwrap();
    builder.string(text(&builder, "not json")).unwrap();
    builder.key(text(&builder, "keep")).unwrap();
    builder.number(42.into()).unwrap();
    builder.end_container().unwrap();
    let value = builder.finish().unwrap();
    assert_eq!(
        serde_json::to_string(value.as_owned()).unwrap(),
        r#"{"$serde_json::private::RawValue":"not json","keep":42}"#
    );
    assert_eq!(
        value.as_owned().as_value()["$serde_json::private::RawValue"],
        "not json"
    );
    assert!(value.as_owned().receipt().matches(value.as_owned()));
}
#[test]
fn deep_literal_construction_uses_iterative_frames_without_global_128_policy() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let fixed = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    for _ in 0..200 {
        builder.start_array().unwrap();
    }
    builder.number(0.into()).unwrap();
    for _ in 0..200 {
        builder.end_container().unwrap();
    }
    let decoded = builder.finish().unwrap();
    let mut value = decoded.as_owned().as_value();
    for _ in 0..200 {
        value = &value.as_array().unwrap()[0];
    }
    assert_eq!(value, &Value::from(0));
    assert!(ledger.peak_requested_bytes() <= 1024 * 1024);
    let retained = decoded.as_owned().retained_heap_bytes();
    assert_eq!(ledger.live_requested_bytes(), fixed + retained);
    drop(decoded);
    assert_eq!(ledger.live_requested_bytes(), fixed);
}
#[test]
fn duplicate_replacement_keeps_first_key_position_and_releases_old_descendant() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    builder.start_object().unwrap();
    builder.key(text(&builder, "same")).unwrap();
    builder.string(text(&builder, &"x".repeat(4096))).unwrap();
    builder.key(text(&builder, "other")).unwrap();
    builder.boolean(true).unwrap();
    let before = ledger.live_requested_bytes();
    builder.key(text(&builder, "same")).unwrap();
    builder.number(7.into()).unwrap();
    assert!(ledger.live_requested_bytes() < before);
    builder.end_container().unwrap();
    let decoded = builder.finish().unwrap();
    assert_eq!(
        serde_json::to_string(decoded.as_owned()).unwrap(),
        r#"{"same":7,"other":true}"#
    );
}
#[test]
fn construction_rejects_foreign_text_and_out_of_order_object_values() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let foreign = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let other = LiteralBuilder::new(&foreign).unwrap();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    builder.start_object().unwrap();
    assert!(matches!(
        builder.key(text(&other, "foreign")),
        Err(JsonDecodeError::ForeignLedger)
    ));
    assert!(matches!(
        builder.number(1.into()),
        Err(JsonDecodeError::ConstructionOrder)
    ));
}
#[test]
fn failed_preallocation_does_not_grow_the_text_backing_or_lose_existing_data() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let probe = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let fixed =
        probe.live_requested_bytes() + size_of::<LiteralBuilder>() + size_of::<LiteralText>();
    drop(probe);
    let ledger = DecodeLedger::new(&authority, fixed + 8).unwrap();
    let builder = LiteralBuilder::new(&ledger).unwrap();
    let mut candidate = text(&builder, "safe");
    let before = ledger.live_requested_bytes();
    assert!(matches!(
        candidate.append(&"x".repeat(1024)),
        Err(JsonDecodeError::Limit { .. })
    ));
    assert_eq!(ledger.live_requested_bytes(), before);
    let (value, _allocation) = candidate.into_parts(&ledger).unwrap();
    assert_eq!(value, "safe");
}

#[test]
fn exact_text_reserves_once_and_preserves_backing_through_final_json() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    let mut text = builder.exact_text(7).unwrap();
    let after_allocation = ledger.live_requested_bytes();
    text.append("abc").unwrap();
    text.append("éß").unwrap();
    assert_eq!(ledger.live_requested_bytes(), after_allocation);
    builder.string(text.finish().unwrap()).unwrap();
    let final_json = builder.finish().unwrap();
    assert_eq!(final_json.as_owned().as_value().as_str(), Some("abcéß"));
    assert!(
        final_json
            .as_owned()
            .receipt()
            .matches(final_json.as_owned())
    );
    drop(final_json);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn exact_text_rejects_changed_length_before_capacity_growth() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let builder = LiteralBuilder::new(&ledger).unwrap();
    let mut text = builder.exact_text(3).unwrap();
    let after_allocation = ledger.live_requested_bytes();
    text.append("ab").unwrap();
    assert!(matches!(
        text.append("cd"),
        Err(JsonDecodeError::TextLength {
            expected: 3,
            actual: 4
        })
    ));
    assert_eq!(ledger.live_requested_bytes(), after_allocation);
    assert!(matches!(
        text.finish(),
        Err(JsonDecodeError::TextLength {
            expected: 3,
            actual: 2
        })
    ));
    drop(builder);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn planned_array_extent_refuses_changed_cardinality_without_growing_the_backing() {
    let authority = crate::model::retention::RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    builder.start_array_exact(2).unwrap();
    builder.null().unwrap();
    builder.boolean(true).unwrap();
    let before_extra = ledger.live_requested_bytes();
    assert!(matches!(
        builder.null(),
        Err(JsonDecodeError::ConstructionOrder)
    ));
    assert_eq!(ledger.live_requested_bytes(), before_extra);
    builder.end_container().unwrap();
    let value = builder.finish().unwrap();
    assert_eq!(
        serde_json::to_string(value.as_owned()).unwrap(),
        "[null,true]"
    );
    drop(value);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    builder.start_array_exact(2).unwrap();
    builder.null().unwrap();
    assert!(matches!(
        builder.end_container(),
        Err(JsonDecodeError::ConstructionOrder)
    ));
    drop(builder);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
