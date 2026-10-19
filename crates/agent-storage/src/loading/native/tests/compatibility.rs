//! Unchanged stock Seed/Value oracles define boundaries; this file is source-only.
use super::{ID, legacy, load};
use crate::loading::{
    held_file::HeldFile,
    json_fields::Key,
    native::{
        Error, legacy_value,
        schema::{Schema, Scope},
        tools,
    },
};
use nocterm_ai::{
    history::{MAX_RESIDENT_BYTES, SavedChat},
    model::{
        decode::Seed, json::deferred::DecodeLedger, retention::RetentionAuthority,
        tools::ClientDisplay,
    },
};
use serde::de::DeserializeSeed;
use serde_json::Value;
fn document(entries: &str, metadata: &str) -> String {
    format!(
        r#"{{"id":"{ID}","metadata":{metadata},"resume":{{"config":{{}},"pending_history":false}},"transcript":{{"entries":{entries}}},"composer":{{}}}}"#
    )
}
const META: &str = r#"{"agent_id":"provider","pinned":false,"updated":17}"#;
fn compare_document(document: &str) {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let mut decoder = serde_json::Deserializer::from_str(document);
    let stock = Seed::<SavedChat>::new(&authority)
        .deserialize(&mut decoder)
        .and_then(|chat| {
            decoder.end()?;
            Ok(chat)
        });
    let actual = load(&format!(
        r#"{{"format":"nocterm.chat","version":2,"document":{document}}}"#
    ));
    assert_eq!(
        actual.is_ok(),
        stock.is_ok(),
        "stock Seed acceptance differs for {document}"
    );
    if let (Ok(actual), Ok(stock)) = (actual, stock) {
        assert_eq!(
            serde_json::to_value(actual.transcript()).unwrap(),
            serde_json::to_value(stock.transcript()).unwrap()
        );
    }
}
#[test]
fn tagged_source_value_expands_raw_before_typed_payload_and_duplicate_checks() {
    compare_document(&document(
        r#"[{"kind":"agent","value":{"$serde_json::private::RawValue":"\"expanded\""}}]"#,
        META,
    ));
    let tool = r#"[{"kind":"tool","value":{"id":"t","title":null,"title":"last","kind":"other","status":"pending","output":[],"locations":[],"arguments":{"presence":"absent"},"result":{"presence":"absent"}}}]"#;
    compare_document(&document(tool, META));
    compare_document(&document(
        "[]",
        r#"{"agent_id":"p","pinned":false,"updated":1,"title":null,"title":"last"}"#,
    ));
    compare_document(&document(
        r#"[{"kind":"agent","kind":"thought","value":"repeated tag"}]"#,
        META,
    ));
}
#[test]
fn owned_present_validates_even_an_absent_supplied_value() {
    let malformed = r#"[{"kind":"tool","value":{"id":"t","title":"old","kind":"other","status":"pending","output":[],"locations":[],"arguments":{"presence":"absent","value":{"$serde_json::private::RawValue":"invalid"}},"result":{"presence":"absent"}}}]"#;
    compare_document(&document(malformed, META));
}
fn controlled_display(body: &str) -> Result<(), Error> {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES)?;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("display.json");
    std::fs::write(&path, format!(r#"{{"client_display":{body}}}"#)).unwrap();
    let mut held = HeldFile::open(&path)?;
    let stamp = held.preflight_source(&ledger)?;
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            Scope::scan(
                reader,
                0,
                Schema::new(&[Key::ClientDisplay], &[Key::ClientDisplay], 1),
            )
        })?
        .transpose()?;
    drop(tools::display::display(
        &plan,
        &mut held,
        Key::ClientDisplay,
        &ledger,
    )?);
    Ok(())
}
#[test]
fn plain_presence_and_owned_outcome_keep_different_value_semantics() {
    for (destination, outcome) in [
        (
            r#"{"presence":"absent","value":null}"#,
            r#"{"presence":"absent","value":{"ordinary":true}}"#,
        ),
        (
            r#"{"presence":"null","value":"unit cannot carry text"}"#,
            r#"{"presence":"absent"}"#,
        ),
        (
            r#"{"presence":"absent","value":{"$serde_json::private::RawValue":"invalid"}}"#,
            r#"{"presence":"absent"}"#,
        ),
        (
            r#"{"presence":"absent"}"#,
            r#"{"presence":"absent","value":{"$serde_json::private::RawValue":"invalid"}}"#,
        ),
        (
            r#"{"presence":"null"}"#,
            r#"{"presence":"value","value":{"$serde_json::private::RawValue":"\"Pending\""}}"#,
        ),
    ] {
        let body = format!(
            r#"{{"version":2,"tool":"x","arguments":null,"destination":{destination},"redacted":{{"presence":"absent"}},"outcome":{outcome}}}"#
        );
        let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
        let mut decoder = serde_json::Deserializer::from_str(&body);
        let stock = Seed::<ClientDisplay>::new(&authority)
            .deserialize(&mut decoder)
            .and_then(|display| {
                decoder.end()?;
                Ok(display)
            });
        assert_eq!(controlled_display(&body).is_ok(), stock.is_ok(), "{body}");
    }
}
fn controlled_value(payload: &str) -> Result<Value, Error> {
    controlled_value_in(payload, false, MAX_RESIDENT_BYTES).map(|(value, _)| value)
}
fn controlled_value_in(
    payload: &str,
    map: bool,
    allowance: usize,
) -> Result<(Value, usize), Error> {
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, allowance)?;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("value.json");
    std::fs::write(&path, format!(r#"{{"rawInput":{payload}}}"#)).unwrap();
    let mut held = HeldFile::open(&path)?;
    let stamp = held.preflight_source(&ledger)?;
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            Scope::scan(
                reader,
                0,
                Schema::new(&[Key::RawInput], &[Key::RawInput], 1),
            )
        })?
        .transpose()?;
    let field = plan.payload().required(Key::RawInput)?;
    let node = legacy_value::authenticated(&plan, &mut held, |_| field.span(), &ledger, map)??;
    let value = node.into_decoded(&ledger)?.as_owned().as_value().clone();
    Ok((value, ledger.peak_requested_bytes()))
}
#[test]
fn raw_expansion_keeps_stock_127_per_invocation_and_no_combined_depth_cap() {
    for depth in [126, 127, 128] {
        let inner = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&inner).unwrap()
        );
        let stock = serde_json::from_str::<Value>(&payload);
        assert_eq!(
            stock.is_ok(),
            depth < 128,
            "pinned stock recursion boundary"
        );
        assert_eq!(controlled_value(&payload).is_ok(), stock.is_ok());
    }
    let inner = format!("{}0{}", "[".repeat(100), "]".repeat(100));
    let marker = format!(
        r#"{{"$serde_json::private::RawValue":{}}}"#,
        serde_json::to_string(&inner).unwrap()
    );
    let outer = format!("{}{}{}", "[".repeat(100), marker, "]".repeat(100));
    assert_eq!(
        controlled_value(&outer).unwrap(),
        serde_json::from_str::<Value>(&outer).unwrap()
    );
}
#[test]
fn scalar_and_unicode_language_matches_pinned_stock_without_token_size_caps() {
    let long = format!("1.{}", "0".repeat(4096));
    for payload in [
        long.as_str(),
        "1e-9999",
        "1e9999",
        "01",
        "-0",
        "18446744073709551616",
        r#""\uD834\uDD1E""#,
        r#""\uD834x""#,
        r#""\uDC00""#,
        r#""é中""#,
    ] {
        let stock = serde_json::from_str::<Value>(payload);
        let actual = controlled_value(payload);
        assert_eq!(actual.is_ok(), stock.is_ok(), "{payload}");
        if let (Ok(actual), Ok(stock)) = (actual, stock) {
            assert_eq!(actual, stock);
        }
    }
}
#[test]
fn original_source_k_counts_ignored_and_defaulted_fields_before_route() {
    let values = format!("[{}null]", "null,".repeat(140_000));
    for source in [
        legacy("[]", &format!(r#", "ignored":{values}"#)),
        format!(r#"{{"version":999,"ignored":{values}}}"#),
    ] {
        assert!(matches!(
            load(&source),
            Err(nocterm_ai::storage::AgentStorageError::Codec(
                nocterm_ai::history::HistoryCodecError::StructuralLimit
            ))
        ));
    }
}
#[test]
fn source_physical_recursion_guard_remains_distinct_from_raw_expansion() {
    for depth in [126, 127, 128] {
        let source = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
        let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
        let actual = crate::loading::literal_reader::original_source(source.as_bytes(), &ledger);
        assert_eq!(
            actual.is_ok(),
            nocterm_ai::history::structural_bytes(source.as_bytes()).is_ok()
        );
    }
}

#[test]
fn overwritten_raw_intermediate_keeps_original_acceptance_at_large_boundaries() {
    for count in [140_000, 1_000_000] {
        let expanded = format!(r#"{{"a":[{}null],"a":null}}"#, "null,".repeat(count - 1));
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap(),
            serde_json::json!({"a":null})
        );
        let rows = format!(
            r#"[{{"kind":"tool","value":{{"toolCallId":"original","title":"old","rawInput":{payload}}}}}]"#
        );
        let bytes = legacy(&rows, "");
        let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
        let original = crate::legacy::decode(&authority, bytes.as_bytes(), ID)
            .expect("unchanged original legacy codec accepts the overwritten intermediate");
        let actual=load(&bytes).expect("controlled public intake must retain original acceptance; do not default an allocation refusal");
        assert_eq!(
            serde_json::to_value(actual.transcript()).unwrap(),
            serde_json::to_value(original.transcript()).unwrap()
        );
    }
}

#[test]
fn discarded_million_children_need_no_final_array_or_string_fact_backing() {
    // 24 MiB cannot admit a million Managed slots; only the tiny winner is built.
    // This is a private parser capacity regression, not a public whole-peak proof.
    let allowance = 24 * 1024 * 1024;
    for item in ["null", r#""""#] {
        let expanded = format!(
            r#"{{"a":[{}{}],"a":null}}"#,
            format!("{item},").repeat(999_999),
            item
        );
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
        let stock = serde_json::from_str::<Value>(&payload).unwrap();
        let (actual, peak) = controlled_value_in(&payload, false, allowance)
            .expect("discarded children must be streamed without per-child output/facts");
        assert_eq!(actual, stock);
        assert_eq!(actual, serde_json::json!({"a":null}));
        assert!(peak <= allowance);
    }
}
#[test]
fn discarded_large_map_is_validated_without_a_unique_key_index() {
    let mut expanded = String::from(r#"{"a":{"#);
    for index in 0..100_000 {
        if index != 0 {
            expanded.push(',');
        }
        expanded.push_str(&format!(r#""key{index}":null"#));
    }
    expanded.push_str(r#"},"a":null}"#);
    let payload = format!(
        r#"{{"$serde_json::private::RawValue":{}}}"#,
        serde_json::to_string(&expanded).unwrap()
    );
    let stock = serde_json::from_str::<Value>(&payload).unwrap();
    let (actual, peak) = controlled_value_in(&payload, false, 24 * 1024 * 1024).unwrap();
    assert_eq!(actual, stock);
    assert!(peak <= 24 * 1024 * 1024);
}
#[test]
fn retained_maps_move_decoded_keys_in_first_position_with_last_value() {
    for payload in [
        r#"{"b":1,"a":{"x":0,"x":2},"\u0062":3}"#,
        r#"{"ordinary":0,"$serde_json::private::RawValue":"invalid"}"#,
        r#"{"$serde_json::private::RawValue":"{\"b\":1,\"a\":2,\"b\":3}"}"#,
    ] {
        let stock = serde_json::from_str::<Value>(payload).unwrap();
        let actual = controlled_value(payload).unwrap();
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&stock).unwrap()
        );
    }
    let payload = r#"{"$serde_json::private::RawValue":"invalid","a":null}"#;
    let stock = serde_json::from_str::<serde_json::Map<String, Value>>(payload).unwrap();
    let (actual, _) = controlled_value_in(payload, true, MAX_RESIDENT_BYTES).unwrap();
    assert_eq!(
        serde_json::to_string(&actual).unwrap(),
        serde_json::to_string(&stock).unwrap()
    );
    assert!(controlled_value_in("null", true, MAX_RESIDENT_BYTES).is_err());
}
#[test]
fn invalid_overwritten_raw_is_never_hidden_by_winner_selection() {
    let invalid = r#"{"$serde_json::private::RawValue":"invalid"}"#;
    for expanded in [
        format!(r#"{{"a":{invalid},"b":0,"a":null}}"#),
        format!(r#"{{"b":0,"a":{invalid},"a":null}}"#),
        r#"{"$serde_json::private::RawValue":0}"#.to_owned(),
        r#"{"$serde_json::private::RawValue":"null","extra":0}"#.to_owned(),
    ] {
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
        assert!(serde_json::from_str::<Value>(&payload).is_err());
        assert!(
            controlled_value(&payload).is_err(),
            "invalid discarded occurrence must be read"
        );
    }
}

#[test]
fn nested_raw_sources_with_pending_siblings_preserve_original_acceptance() {
    let mut expanded = format!("{}null", " ".repeat(4 * 1024 * 1024));
    for _ in 0..16 {
        expanded = format!(
            r#"{{"x":{{"$serde_json::private::RawValue":{}}},"y":null}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
    }
    let payload = format!(
        r#"{{"$serde_json::private::RawValue":{}}}"#,
        serde_json::to_string(&expanded).unwrap()
    );
    let rows = format!(
        r#"[{{"kind":"tool","value":{{"toolCallId":"t","title":"old","rawInput":{payload}}}}}]"#
    );
    let bytes = legacy(&rows, "");
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let original = crate::legacy::decode(&authority, bytes.as_bytes(), ID)
        .expect("the original codec accepts nested raw sources with pending siblings");
    let nocterm_ai::thread::Entry::Tool(original) = &original.transcript().entries[0] else {
        panic!("original tool row");
    };
    let expected = original.arguments.as_ref().unwrap().as_value();
    // Private parser acceptance under its actual ledger; whole public-phase
    // allocation instrumentation is a separate requirement.
    let (actual, peak) = controlled_value_in(&payload, false, MAX_RESIDENT_BYTES)
        .expect("accepted nested raw sources must not require all decoded caches at once");
    assert_eq!(&actual, expected);
    assert!(peak <= MAX_RESIDENT_BYTES);
}

#[test]
fn raw_source_lenses_need_no_whole_decoded_text_allocation() {
    let mut expanded = format!("{}null", " ".repeat(1024 * 1024));
    for _ in 0..8 {
        expanded = format!(
            r#"{{"x":{{"$serde_json::private::RawValue":{}}},"y":null}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
    }
    let payload = format!(
        r#"{{"$serde_json::private::RawValue":{}}}"#,
        serde_json::to_string(&expanded).unwrap()
    );
    let expected = serde_json::from_str::<Value>(&payload).unwrap();
    let (actual, peak) = controlled_value_in(&payload, false, 256 * 1024)
        .expect("a source lens must operate below even one decoded source's length");
    assert_eq!(
        serde_json::to_string(&actual).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
    assert!(peak <= 256 * 1024);
}
#[test]
fn raw_lenses_restore_parent_positions_after_scalar_and_unicode_siblings() {
    for inner in [
        r#""\uD834\uDD1E""#,
        r#""é中""#,
        "-0",
        "1.25e-3",
        "null",
        "true",
    ] {
        let child = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(inner).unwrap()
        );
        let expanded = format!(r#"{{"before":0,"child":{child},"after":[{child},3]}}"#);
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
        let stock = serde_json::from_str::<Value>(&payload).unwrap();
        let actual = controlled_value(&payload).unwrap();
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&stock).unwrap()
        );
    }
}
#[test]
fn malformed_nested_raw_unicode_preserves_stock_rejection() {
    for inner in [r#""\uD834x""#, r#""\uDC00""#, r#""\uZZZZ""#, "[null,]"] {
        let child = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(inner).unwrap()
        );
        let expanded = format!(r#"{{"old":{child},"old":null}}"#);
        let payload = format!(
            r#"{{"$serde_json::private::RawValue":{}}}"#,
            serde_json::to_string(&expanded).unwrap()
        );
        assert!(serde_json::from_str::<Value>(&payload).is_err());
        let error = controlled_value(&payload).unwrap_err();
        assert!(
            error.defaultable(),
            "malformed decoded Unicode is an original semantic fallback"
        );
    }
}
#[test]
fn lens_allocation_refusal_is_never_a_semantic_default() {
    let payload = r#"{"$serde_json::private::RawValue":"null"}"#;
    let error = controlled_value_in(payload, false, 1024).unwrap_err();
    assert!(!error.defaultable());
}
