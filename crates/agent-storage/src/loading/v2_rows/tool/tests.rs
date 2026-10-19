//! Literal tool arguments and exact presence grammar through held typed row path.
use super::*;
use crate::loading::v2_rows::{RowReadError, read_row};
use nocterm_ai::thread::Entry;
use serde_json::{Value, json};
use std::fs;
pub(super) fn tool(arguments: Value) -> Value {
    json!({"kind":"tool","value":{"id":"original","title":"literal","name":null,"kind":"other","status":"pending","output":[],"locations":[],"arguments":arguments,"result":{"presence":"absent"},"extensions":null,"client_display":null}})
}
pub(super) fn native(bytes: &[u8], ledger: &DecodeLedger) -> Result<DecodedRow, RowReadError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("row.json");
    fs::write(&path, bytes).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| V2Plan::scan(reader, 0, Scope::Entry))
        .unwrap()
        .transpose()
        .unwrap();
    let row = read_row(&plan, &mut held, ledger)?;
    assert_eq!(fs::read(path).unwrap(), bytes);
    Ok(row)
}
#[test]
fn ordinary_tool_row_matches_actual_entry_and_preserves_literal_null_presence() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for arguments in [
        json!({"presence":"absent"}),
        json!({"presence":"supplied","value":null}),
        json!({"presence":"supplied","value":{"first":[1,true],"last":"literal"}}),
    ] {
        let bytes = serde_json::to_vec(&tool(arguments)).unwrap();
        let expected: Entry = crate::history_fixtures::decoded(&authority, &bytes).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let initial = ledger.live_requested_bytes();
        let row = native(&bytes, &ledger).unwrap();
        assert_eq!(row.as_entry(), &expected);
        drop(row);
        assert_eq!(ledger.live_requested_bytes(), initial);
    }
}
#[test]
fn literal_optional_presence_matches_original_absent_unit_and_positional_semantics() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for arguments in [
        json!({"presence":"absent"}),
        json!({"presence":"absent","value":null}),
        json!(["absent", null]),
        json!(["supplied", null]),
        json!(["absent"]),
        json!(null),
        json!({"value":null}),
        json!({"presence":"supplied"}),
        json!({"presence":"absent","value":7}),
    ] {
        let bytes = serde_json::to_vec(&tool(arguments.clone())).unwrap();
        let stock = crate::history_fixtures::decoded::<Entry>(&authority, &bytes);
        let result = native(&bytes, &DecodeLedger::new(&authority, 1024 * 1024).unwrap());
        assert_eq!(result.is_ok(), stock.is_ok(), "{arguments}");
        if let (Ok(actual), Ok(expected)) = (result, stock) {
            assert_eq!(actual.as_entry(), &expected);
        }
    }
}
#[test]
fn custom_option_fields_are_required_while_plain_option_fields_may_be_missing() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original = tool(json!({"presence":"absent"}));
    for (field, accepted) in [
        ("arguments", false),
        ("result", false),
        ("client_display", false),
        ("name", true),
        ("extensions", true),
    ] {
        let mut value = original.clone();
        value["value"].as_object_mut().unwrap().remove(field);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            crate::history_fixtures::decoded::<Entry>(&authority, &bytes).is_ok(),
            accepted,
            "stock {field}"
        );
        assert_eq!(
            native(&bytes, &DecodeLedger::new(&authority, 1024 * 1024).unwrap()).is_ok(),
            accepted,
            "native {field}"
        );
    }
}
#[test]
fn controlled_tool_row_preserves_200_level_literal_payload_and_reserved_first_key_data() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let prefix = r#"{"kind":"tool","value":{"id":"original","title":"deep","kind":"other","status":"pending","output":[],"locations":[],"arguments":{"presence":"supplied","value":"#;
    let payload = format!(
        "{}{{\"$serde_json::private::RawValue\":\"literal data\"}}{}",
        "[".repeat(200),
        "]".repeat(200)
    );
    let source = format!(
        "{prefix}{payload}{}",
        r#"},"result":{"presence":"absent"},"client_display":null}}"#
    );
    let row = native(
        source.as_bytes(),
        &DecodeLedger::new(&authority, 1024 * 1024).unwrap(),
    )
    .unwrap();
    let Entry::Tool(call) = row.as_entry() else {
        panic!("tool")
    };
    let mut value = call.arguments.as_ref().unwrap().as_value();
    for _ in 0..200 {
        value = &value.as_array().unwrap()[0];
    }
    assert_eq!(value["$serde_json::private::RawValue"], "literal data");
}
#[test]
fn valid_compound_fields_keep_supported_display_and_explicit_remaining_gaps() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for (key, value) in [
        (
            "output",
            json!([{"kind":"terminal_reference","value":{"id":"terminal","extensions":null}}]),
        ),
        (
            "locations",
            json!([{"path":"x","line":null,"extensions":null}]),
        ),
        ("extensions", json!({"x":1})),
        (
            "client_display",
            json!({"version":1,"tool":"tool","arguments":null,"destination":{"presence":"absent"},"redacted":{"presence":"absent"},"outcome":{"presence":"absent"}}),
        ),
    ] {
        let mut value_entry = tool(json!({"presence":"absent"}));
        value_entry["value"][key] = value;
        let bytes = serde_json::to_vec(&value_entry).unwrap();
        assert!(
            crate::history_fixtures::decoded::<Entry>(&authority, &bytes).is_ok(),
            "actual valid native case {key}"
        );
        if key == "client_display" {
            let expected: Entry = crate::history_fixtures::decoded(&authority, &bytes).unwrap();
            let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
            let before = ledger.live_requested_bytes();
            let row = native(&bytes, &ledger).unwrap();
            assert_eq!(row.as_entry(), &expected);
            assert!(ledger.live_requested_bytes() > before);
            drop(row);
            assert_eq!(ledger.live_requested_bytes(), before);
        } else {
            assert!(
                matches!(
                    native(&bytes, &DecodeLedger::new(&authority, 1024 * 1024).unwrap()),
                    Err(RowReadError::Tool(ToolReadError::Incomplete(_)))
                ),
                "explicit incomplete {key}"
            );
        }
    }
}
