//! Complete persisted client-display grammar through real held-row construction.
use super::super::tests::{native, tool};
use super::*;
use nocterm_ai::thread::Entry;
use serde_json::{Value, json};
fn display() -> Value {
    json!({"version":1,"tool":" original tool ","arguments":null,"destination":{"presence":"absent"},"redacted":{"presence":"absent"},"outcome":{"presence":"absent"}})
}
fn bytes(display: Value) -> Vec<u8> {
    let mut entry = tool(json!({"presence":"absent"}));
    entry["value"]["client_display"] = display;
    serde_json::to_vec(&entry).unwrap()
}
fn compare(authority: &nocterm_ai::model::retention::RetentionAuthority, bytes: &[u8]) {
    let expected = crate::history_fixtures::decoded::<Entry>(authority, bytes);
    let ledger = DecodeLedger::new(authority, 1024 * 1024).unwrap();
    let before = ledger.live_requested_bytes();
    let actual = native(bytes, &ledger);
    assert_eq!(
        actual.is_ok(),
        expected.is_ok(),
        "{:?}",
        String::from_utf8_lossy(bytes)
    );
    if let (Ok(actual), Ok(expected)) = (actual, expected) {
        assert_eq!(actual.as_entry(), &expected);
        assert_eq!(
            serde_json::to_vec(actual.as_entry()).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        drop(actual);
    }
    assert_eq!(ledger.live_requested_bytes(), before);
}
#[test]
fn actual_display_wire_preserves_full_u8_versions_and_all_presence_outcome_branches() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for version in [0, 1, 255] {
        let mut value = display();
        value["version"] = json!(version);
        compare(&authority, &bytes(value));
    }
    let mut ordered = display();
    ordered["arguments"] = json!({"second":2,"first":1});
    compare(&authority, &bytes(ordered));
    for (key, values) in [
        (
            "destination",
            vec![
                json!({"presence":"absent"}),
                json!({"presence":"null"}),
                json!({"presence":"value","value":"  literal 日本  "}),
            ],
        ),
        (
            "redacted",
            vec![
                json!({"presence":"absent"}),
                json!({"presence":"null"}),
                json!({"presence":"value","value":false}),
                json!({"presence":"value","value":true}),
            ],
        ),
        (
            "outcome",
            vec![
                json!({"presence":"absent"}),
                json!({"presence":"null"}),
                json!({"presence":"value","value":"Pending"}),
                json!({"presence":"value","value":{"Pending":null}}),
                json!({"presence":"value","value":{"Ok":null}}),
                json!({"presence":"value","value":{"Ok":{"keep":[true,null]}}}),
                json!({"presence":"value","value":{"Err":" literal failure "}}),
            ],
        ),
    ] {
        for branch in values {
            let mut value = display();
            value[key] = branch;
            compare(&authority, &bytes(value));
        }
    }
}
#[test]
fn missing_display_fields_and_invalid_presence_values_match_original_serde() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for key in [
        "version",
        "tool",
        "arguments",
        "destination",
        "redacted",
        "outcome",
    ] {
        let mut value = display();
        value.as_object_mut().unwrap().remove(key);
        let input = bytes(value);
        assert!(crate::history_fixtures::decoded::<Entry>(&authority, &input).is_err());
        compare(&authority, &input);
    }
    for (key, value) in [
        ("version", json!(-1)),
        ("version", json!(256)),
        ("version", json!(1.25)),
        ("version", json!("1")),
        ("tool", json!(false)),
        ("destination", json!({"presence":"value","value":1})),
        ("destination", json!({"presence":"absent","value":{}})),
        ("redacted", json!({"presence":"value","value":"false"})),
        ("redacted", json!({"presence":"value"})),
        ("outcome", json!({"presence":"value","value":"Ok"})),
        ("outcome", json!({"presence":"value","value":{"Err":null}})),
        (
            "outcome",
            json!({"presence":"value","value":{"Pending":{}}}),
        ),
        ("outcome", json!({"presence":"future"})),
    ] {
        let mut base = display();
        base[key] = value;
        compare(&authority, &bytes(base));
    }
}
#[test]
fn display_sequence_arity_duplicate_and_external_enum_shapes_match_original_serde() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for sequence in [
        json!([1,"tool",null,{"presence":"absent"},{"presence":"absent"},{"presence":"absent"}]),
        json!([1,"tool",null,["null",null],["value",false],["value",{"Ok":null}]]),
        json!([1,"tool",null,{"presence":"absent"},{"presence":"absent"}]),
        json!([1,"tool",null,{"presence":"absent"},{"presence":"absent"},{"presence":"absent"},null]),
    ] {
        compare(&authority, &bytes(sequence));
    }
    for (key, raw) in [
        ("destination", r#"["absent"]"#),
        ("redacted", r#"["null",null]"#),
        (
            "destination",
            r#"{"presence":"value","value":"x","presence":"value"}"#,
        ),
        (
            "outcome",
            r#"{"presence":"value","value":{"Ok":null,"Ok":null}}"#,
        ),
        (
            "outcome",
            r#"{"presence":"value","value":{"Ok":null,"Err":"x"}}"#,
        ),
    ] {
        let mut value = display();
        value[key] = json!("__RAW__");
        let input = String::from_utf8(bytes(value))
            .unwrap()
            .replace("\"__RAW__\"", raw);
        compare(&authority, input.as_bytes());
    }
    let input = String::from_utf8(bytes(display())).unwrap().replace(
        "\"tool\":\" original tool \"",
        "\"tool\":\"first\",\"tool\":\"second\"",
    );
    // Original Entry decodes Tagged.value through stock Value before ToolRecord;
    // duplicate nested object members therefore keep the last original value.
    let original = crate::history_fixtures::decoded::<Entry>(&authority, input.as_bytes()).unwrap();
    let Entry::Tool(call) = &original else {
        panic!("original tool")
    };
    assert_eq!(call.client_display.as_ref().unwrap().tool, "second");
    compare(&authority, input.as_bytes());
}
#[test]
fn controlled_display_retains_deep_literal_arguments_and_success_payload_without_raw_expansion() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let mut value = display();
    value["version"] = json!(255);
    value["arguments"] = json!("__LITERAL__");
    value["outcome"] = json!({"presence":"value","value":{"Ok":"__LITERAL__"}});
    let payload = format!(
        "{}{{\"$serde_json::private::RawValue\":\"literal data\"}}{}",
        "[".repeat(200),
        "]".repeat(200)
    );
    let input = String::from_utf8(bytes(value))
        .unwrap()
        .replace("\"__LITERAL__\"", &payload);
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let before = ledger.live_requested_bytes();
    let row = native(input.as_bytes(), &ledger).unwrap();
    let Entry::Tool(call) = row.as_entry() else {
        panic!("tool")
    };
    let display = call.client_display.as_ref().unwrap();
    assert_eq!(display.version, 255);
    let Presence::Value(nocterm_ai::model::tools::DisplayOutcome::Succeeded(result)) =
        &display.outcome
    else {
        panic!("success")
    };
    for root in [display.arguments.as_value(), result.as_value()] {
        let mut value = root;
        for _ in 0..200 {
            value = &value.as_array().unwrap()[0];
        }
        assert_eq!(value["$serde_json::private::RawValue"], "literal data");
    }
    drop(row);
    assert_eq!(ledger.live_requested_bytes(), before);
}
