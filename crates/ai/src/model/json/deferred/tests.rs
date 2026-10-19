use super::*;
use serde_json::Value;
fn decode(text: &str) -> Result<DecodedJson, JsonDecodeError> {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16 * 1024 * 1024)?;
    DeferredJson::from_owned_text(text.to_owned(), &ledger)?.materialize(&ledger)
}
#[test]
fn deferred_numbers_unicode_and_order_match_stock_value() {
    for text in [
        r#"{"z":1,"a":[null,true,-9223372036854775808,18446744073709551615,3.141e-21,"界\ud83d\ude00\u0000"]}"#,
        r#"{"z":1,"a":2,"z":3}"#,
        r#"[1e-400,-0.0,123456789012345678901234567890]"#,
    ] {
        let expected: Value = serde_json::from_str(text).unwrap();
        let actual = decode(text).unwrap();
        assert_eq!(actual.as_owned().as_value(), &expected);
        assert_eq!(
            serde_json::to_vec(actual.as_owned()).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert!(actual.as_owned().receipt().matches(actual.as_owned()));
    }
}
#[test]
fn deferred_raw_expansion_is_first_key_only_and_nested() {
    for text in [
        r#"{"$serde_json::private::RawValue":"[null,{\"x\":2}]"}"#,
        r#"{"$serde_json::private::RawValue":"{\"$serde_json::private::RawValue\":\"42\"}"}"#,
        r#"{"ordinary":1,"$serde_json::private::RawValue":"not json"}"#,
        r#"{"$serde_json::private::Raw\u0056alue":"42"}"#,
    ] {
        let expected: Value = serde_json::from_str(text).unwrap();
        assert_eq!(decode(text).unwrap().as_owned().as_value(), &expected);
    }
}
#[test]
fn overwritten_raw_errors_remain_errors_after_value_replacement() {
    for text in [
        r#"{"x":{"$serde_json::private::RawValue":"not json"},"x":null}"#,
        r#"{"x":[{"$serde_json::private::RawValue":"not json"}],"x":null}"#,
        r#"{"$serde_json::private::RawValue":"null","extra":1}"#,
        r#"{"$serde_json::private::RawValue":null}"#,
        r#"{"$serde_json::private::RawValue":"null","$serde_json::private::RawValue":"42"}"#,
    ] {
        assert!(serde_json::from_str::<Value>(text).is_err());
        assert!(decode(text).is_err());
    }
}
#[test]
fn overwritten_large_raw_array_is_validated_without_a_value_array() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let raw = format!("[{}null]", "null,".repeat(40_000));
    let text = format!(
        r#"{{"x":{{"$serde_json::private::RawValue":{}}},"x":null}}"#,
        serde_json::to_string(&raw).unwrap()
    );
    let expected: Value = serde_json::from_str(&text).unwrap();
    let ledger = DecodeLedger::new(&authority, 2 * 1024 * 1024).unwrap();
    let result = DeferredJson::from_owned_text(text, &ledger)
        .unwrap()
        .materialize(&ledger)
        .unwrap();
    assert_eq!(result.as_owned().as_value(), &expected);
    // A surviving Vec<Value> alone would exceed this entire controlled budget.
    assert!(40_001 * std::mem::size_of::<Value>() > 2 * 1024 * 1024);
    // Stock parser scratch is explicitly NOT part of this ledger yet.
}
#[test]
fn failed_reservation_is_typed_and_releases_partial_construction() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let fixed = ledger.live_requested_bytes();
    let error = DeferredJson::from_owned_text("x".repeat(8192), &ledger).unwrap_err();
    assert!(matches!(error, JsonDecodeError::Limit { .. }));
    assert_eq!(ledger.live_requested_bytes(), fixed);
}
