//! Current-stream literal JSON equivalence and preallocation behavior.
use super::*;
use std::io::Cursor;
fn decoded(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    input: &[u8],
) -> Result<DecodedJson, LiteralReadError> {
    decode_retained(
        Cursor::new(input),
        &DecodeLedger::new(authority, 4 * 1024 * 1024).unwrap(),
    )
}
#[test]
fn literal_stream_matches_stock_leaf_and_structural_semantics_for_ordinary_json() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for input in [
        r#"null"#,
        r#"true"#,
        r#"false"#,
        r#"-1"#,
        r#"18446744073709551615"#,
        r#"1.125e-5"#,
        r#"[]"#,
        r#"{}"#,
        r#"{"duplicate":1,"duplicate":2,"nested":[null,true,"日本\uD834\uDD1E"]}"#,
        r#""\\\"\b\f\n\r\t/""#,
    ] {
        let expected: serde_json::Value = serde_json::from_str(input).unwrap();
        let actual = decoded(&authority, input.as_bytes()).unwrap();
        assert_eq!(actual.as_owned().as_value(), &expected, "{input}");
    }
}
#[test]
fn malformed_structure_unicode_and_scalar_tokens_are_not_accepted() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for input in [
        "",
        " ",
        "[1,]",
        "{\"x\":}",
        "{\"x\" 1}",
        "[}",
        "1 2",
        "01",
        "NaN",
        "1e999999",
        r#""\uD800""#,
        r#""\uDC00""#,
        r#"{"x":1,}"#,
    ] {
        assert!(
            serde_json::from_str::<serde_json::Value>(input).is_err(),
            "stock {input}"
        );
        assert!(
            decoded(&authority, input.as_bytes()).is_err(),
            "candidate {input}"
        );
    }
    assert!(decoded(&authority, b"\"\xff\"").is_err());
}
#[test]
fn retained_literal_reader_handles_200_levels_without_raw_encoding_or_parser_recursion() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let input = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let mut stock = serde_json::Deserializer::from_str(&input);
    <serde::de::IgnoredAny as serde::Deserialize>::deserialize(&mut stock).unwrap();
    stock.end().unwrap();
    let actual = decoded(&authority, input.as_bytes()).unwrap();
    let mut value = actual.as_owned().as_value();
    for _ in 0..200 {
        value = &value.as_array().unwrap()[0];
    }
    assert_eq!(value, &serde_json::Value::from(0));
    assert_eq!(serde_json::to_string(actual.as_owned()).unwrap(), input);
}
#[test]
fn reserved_first_key_and_duplicate_reserved_key_remain_literal_v2_data() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let input=br#"{"$serde_json::private::RawValue":"not json","$serde_json::private::RawValue":"still data","keep":7}"#;
    let actual = decoded(&authority, input).unwrap();
    assert_eq!(
        serde_json::to_string(actual.as_owned()).unwrap(),
        r#"{"$serde_json::private::RawValue":"still data","keep":7}"#
    );
}
#[test]
fn valid_long_numeric_exponent_is_streamed_without_a_new_token_length_cap() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let input = format!("0e+{}1", "0".repeat(128 * 1024));
    let expected: serde_json::Value = serde_json::from_str(&input).unwrap();
    let actual = decoded(&authority, input.as_bytes()).unwrap();
    assert_eq!(actual.as_owned().as_value(), &expected);
}
#[cfg(unix)]
#[test]
fn real_native_read_error_keeps_its_original_concrete_io_cause() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    use std::error::Error as _;
    let directory = tempfile::tempdir().unwrap();
    let source = std::fs::File::open(directory.path()).unwrap();
    let error = decode_retained(source, &DecodeLedger::new(&authority, 1024 * 1024).unwrap())
        .err()
        .unwrap();
    let LiteralReadError::Source(source) = &error else {
        panic!("typed native cause")
    };
    assert_eq!(source.as_ref().kind(), std::io::ErrorKind::IsADirectory);
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .is::<std::io::Error>()
    );
}
#[test]
fn current_decoded_string_capacity_is_guarded_without_trusting_a_former_length() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let baseline = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    drop(decode_retained(Cursor::new(br#""\u0061\u0061\u0061\u0061""#), &baseline).unwrap());
    let allowance = baseline.peak_requested_bytes();
    let ledger = DecodeLedger::new(&authority, allowance).unwrap();
    let result = decode_retained(Cursor::new(br#""abcdefghijklmnopqrstuvwx""#), &ledger);
    assert!(matches!(
        result,
        Err(LiteralReadError::Build(JsonDecodeError::Limit { .. }))
    ));
    assert!(ledger.peak_requested_bytes() <= allowance);
}

mod large_text;
