use super::*;
use serde_json::{Map, json};
#[test]
fn sparse_untracked_maps_and_arrays_publish_only_canonical_retained_reserve() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut sparse = Map::with_capacity(100_000);
    sparse.insert("keep".into(), json!(null));
    let mut array = Vec::with_capacity(100_000);
    array.push(Value::Object(sparse));
    let owned = OwnedJson::from_untracked(&authority, Value::Array(array)).unwrap();
    assert_eq!(owned, json!([{"keep":null}]));
    assert!(owned.retained_heap_bytes() < 1024);
    let clone = owned.clone();
    assert!(clone.shares_storage(&owned));
    assert!(owned.receipt().matches(&clone));
    let replacement = OwnedJson::from_untracked(&authority, owned.as_value().clone()).unwrap();
    assert_eq!(owned, replacement);
    assert!(!owned.receipt().matches(&replacement));
}
#[test]
fn large_sparse_string_reserve_is_normalized_and_receipt_survives_weak_expiry() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut text = String::with_capacity(1_000_000);
    text.push_str("literal");
    let owned = OwnedJson::from_untracked(&authority, Value::String(text)).unwrap();
    assert!(owned.retained_heap_bytes() < 1024);
    let receipt = owned.receipt().clone();
    assert_eq!(
        receipt.retained_identity_bytes(),
        Some(owned.retained_heap_bytes())
    );
    drop(owned);
    assert_eq!(
        receipt.retained_identity_bytes(),
        crate::model::json::OwnedJson::identity_allocation_bytes()
    );
}
#[test]
fn nested_order_duplicates_numbers_null_and_raw_sentinel_follow_stock_serde() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    for text in [
        r#"{"z":[null,false,1.25],"a":{"b":1,"a":2}}"#,
        r#"{"z":1,"a":false,"z":3}"#,
        r#"{"$serde_json::private::RawValue":"{\"z\":1,\"a\":[false,null]}"}"#,
    ] {
        let expected: Value = serde_json::from_str(text).unwrap();
        let owned = <OwnedJson as crate::model::decode::Decode>::decode(
            &authority,
            &mut serde_json::Deserializer::from_str(text),
        )
        .unwrap();
        assert_eq!(owned, expected);
        assert_eq!(
            serde_json::to_vec(&owned).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
    }
}
#[test]
fn canonical_object_receipt_uses_requested_dense_slots_with_hash_rounding() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    for count in 1..1000 {
        let expected = containers::object_bytes(count).unwrap();
        assert!(expected <= 156 * count);
        let values: Map<_, _> = (0..count)
            .map(|index| (index.to_string(), Value::Null))
            .collect();
        let owned = OwnedJson::from_untracked(&authority, Value::Object(values)).unwrap();
        let key_bytes: usize = owned
            .as_object()
            .unwrap()
            .keys()
            .map(|key| key.capacity())
            .sum();
        assert_eq!(
            owned.retained_heap_bytes(),
            expected
                + key_bytes
                + crate::model::json::OwnedJson::identity_allocation_bytes().unwrap()
        );
    }
}

#[test]
fn cloned_controlled_json_keeps_payload_credit_and_weak_identity_keeps_inline_credit() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    use crate::model::json::deferred::{DecodeLedger, literal::LiteralBuilder};
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    let mut text = builder.text().unwrap();
    text.append("retained controlled payload").unwrap();
    builder.string(text).unwrap();
    let decoded = builder.finish().unwrap();
    let alias = decoded.as_owned().clone();
    let weak_receipt = alias.receipt().clone();
    let with_payload = ledger.live_requested_bytes();
    drop(decoded);
    assert_eq!(
        alias.as_value().as_str(),
        Some("retained controlled payload")
    );
    assert_eq!(
        ledger.live_requested_bytes(),
        with_payload,
        "a surviving real owner must not return payload credit to the same ledger"
    );
    drop(alias);
    let inline = weak_receipt.retained_identity_bytes().unwrap();
    assert_eq!(
        ledger.live_requested_bytes(),
        baseline + inline,
        "expired real weak identity retains only authentic inline allocation credit"
    );
    drop(weak_receipt);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
