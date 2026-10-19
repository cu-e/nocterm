use super::*;
use std::io::Cursor;
fn read(encoded: &[u8], width: usize) -> Result<Vec<u8>, StringReadError> {
    let mut cursor = JsonStringReader::new(Cursor::new(encoded), encoded.len())?;
    let mut bytes = Vec::new();
    let mut chunk = vec![0; width];
    loop {
        let length = cursor.read(&mut chunk).map_err(StringReadError::source)?;
        if length == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..length]);
    }
}
#[test]
fn string_cursor_matches_stock_escapes_unicode_and_surrogate_runs() {
    for body in [
        "",
        r#"\"\\\/\b\f\n\r\t"#,
        r"\u0000\u007f\u00e9",
        "界😀",
        r"\uD834\uDD1E",
        r"\u0041\uD800\uDC00",
        r"\uD800\uDC00\u0041",
        r"\uD800",
        r"\uDC00",
        r"\uD800\uD800\uDC00",
        r"\uZZZZ",
        r"\x",
        "\n",
        "\r",
        "\t",
    ] {
        let encoded = format!("\"{body}\"");
        let expected = serde_json::from_str::<String>(&encoded);
        for width in 1..=9 {
            let actual = read(encoded.as_bytes(), width);
            match &expected {
                Ok(value) => assert_eq!(actual.unwrap(), value.as_bytes(), "{encoded:?}/{width}"),
                Err(_) => assert!(actual.is_err(), "{encoded:?}/{width}"),
            }
        }
    }
}
#[test]
fn string_cursor_delegates_hex_and_surrogate_acceptance_to_stock() {
    for value in (0..=u16::MAX).step_by(17) {
        let encoded = format!("\"\\u{value:04x}\"");
        let expected = serde_json::from_str::<String>(&encoded);
        match expected {
            Ok(value) => assert_eq!(read(encoded.as_bytes(), 3).unwrap(), value.as_bytes()),
            Err(_) => assert!(read(encoded.as_bytes(), 3).is_err()),
        }
    }
    for first in [0xd800u16, 0xdbff] {
        for second in [0xdc00u16, 0xdfff] {
            let encoded = format!("\"\\u{first:04x}\\u{second:04x}\"");
            let expected: String = serde_json::from_str(&encoded).unwrap();
            assert_eq!(read(encoded.as_bytes(), 1).unwrap(), expected.as_bytes());
        }
    }
}
#[test]
fn string_cursor_rejects_invalid_literal_utf8_and_incomplete_sequences() {
    for body in [
        &[0xff][..],
        &[0xc0, 0x80],
        &[0xed, 0xa0, 0x80],
        &[0xf4, 0x90, 0x80, 0x80],
        &[0xe2, 0x82],
    ] {
        let mut encoded = vec![b'"'];
        encoded.extend_from_slice(body);
        encoded.push(b'"');
        assert!(serde_json::from_slice::<String>(&encoded).is_err());
        assert!(read(&encoded, 1).is_err());
    }
}
#[test]
fn string_cursor_stops_at_original_closing_quote_without_consuming_following_value() {
    let encoded = br#""a\"b\\c",42"#;
    let mut cursor = JsonStringReader::new(Cursor::new(encoded), encoded.len()).unwrap();
    let mut value = Vec::new();
    cursor.read_to_end(&mut value).unwrap();
    assert_eq!(value, b"a\"b\\c");
    let source = cursor.into_inner();
    assert_eq!(&encoded[source.position() as usize..], b",42");
}
#[test]
fn string_cursor_enforces_each_virtual_source_bound() {
    let mut cursor = JsonStringReader::new(Cursor::new(br#""abcd""#), 3).unwrap();
    let mut output = Vec::new();
    let error = cursor.read_to_end(&mut output).unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<StringReadError>(),
        Some(StringReadError::EncodedLimit)
    ));
}
#[test]
fn nested_string_cursors_feed_stock_json_without_materializing_either_string() {
    let payload = r#"{"answer":"界","n":42,"kept":[true,null]}"#;
    let first = serde_json::to_vec(payload).unwrap();
    let second = serde_json::to_vec(std::str::from_utf8(&first).unwrap()).unwrap();
    let outer = JsonStringReader::new(Cursor::new(&second), second.len()).unwrap();
    let inner = JsonStringReader::new(outer, first.len()).unwrap();
    let actual: serde_json::Value = serde_json::from_reader(inner).unwrap();
    let expected: serde_json::Value = serde_json::from_str(payload).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn nested_semantic_failures_do_not_become_native_source_failures() {
    let first = br#""\uD800""#;
    let second = serde_json::to_vec(std::str::from_utf8(first).unwrap()).unwrap();
    let outer = JsonStringReader::new(Cursor::new(&second), second.len()).unwrap();
    let mut inner = JsonStringReader::new(outer, first.len()).unwrap();
    assert!(inner.read_to_end(&mut Vec::new()).is_err());
    assert!(inner.take_source_failure().is_none());
}
#[test]
fn native_read_error_retains_its_concrete_original_cause() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("write-only");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    let error = match JsonStringReader::new(file, 32) {
        Err(error) => error,
        Ok(_) => panic!("write-only source cannot supply an opening quote"),
    };
    let StringReadError::Source(source) = error else {
        panic!("native source cause")
    };
    assert!(source.as_ref().raw_os_error().is_some());
    use std::error::Error as _;
    assert!(
        source
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .is_some()
    );
}

#[test]
fn managed_units_match_stock_without_retaining_success_scratch() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    for source in [
        br#""\n""#.as_slice(),
        br#""\u0000""#,
        br#""\u00e9""#,
        br#""\ud800\udc00""#,
        br#""\udbff\udfff""#,
    ] {
        let expected: String = serde_json::from_slice(source).unwrap();
        let mut cursor =
            JsonStringReader::new_managed(Cursor::new(source), source.len(), &ledger).unwrap();
        let mut actual = String::new();
        while let Some(fragment) = cursor.next_fragment().unwrap() {
            actual.push_str(fragment);
            assert_eq!(ledger.live_requested_bytes(), baseline);
        }
        assert_eq!(actual, expected);
    }
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn managed_stock_unit_error_keeps_original_cause_identity_until_last_clone() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    use std::error::Error as _;
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let baseline = ledger.live_requested_bytes();
    let lexical = br#""\ud800\u0000""#;
    let expected = serde_json::from_slice::<String>(lexical).unwrap_err();
    let error = match unit::parse(lexical, Some(&ledger)) {
        Ok(_) => panic!("stock rejects non-low surrogate"),
        Err(error) => error,
    };
    let StringReadError::ManagedJson(failure) = error else {
        panic!("guarded stock cause")
    };
    let original = failure
        .source()
        .unwrap()
        .downcast_ref::<serde_json::Error>()
        .unwrap();
    assert_eq!(original.classify(), expected.classify());
    assert_eq!(
        (original.line(), original.column()),
        (expected.line(), expected.column())
    );
    assert_eq!(original.to_string(), expected.to_string());
    let clone = failure.clone();
    assert!(std::ptr::eq(
        failure.source().unwrap(),
        clone.source().unwrap()
    ));
    let held = ledger.live_requested_bytes();
    assert!(held > baseline);
    drop(failure);
    assert_eq!(ledger.live_requested_bytes(), held);
    drop(clone);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn managed_unit_refuses_before_stock_constructor_and_does_not_default() {
    use nocterm_ai::model::{json::deferred::DecodeLedger, retention::RetentionAuthority};
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let probe = DecodeLedger::new(&authority, 65536).unwrap();
    let fixed = probe.live_requested_bytes();
    drop(probe);
    let ledger = DecodeLedger::new(&authority, fixed + 7).unwrap();
    let result = unit::parse(br#""\ud800\udc00""#, Some(&ledger));
    assert!(matches!(result, Err(StringReadError::Admission(_))));
    assert_eq!(ledger.live_requested_bytes(), fixed);
}
