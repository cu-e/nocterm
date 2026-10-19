//! General nested literal facts use actual held source spans and stock Unicode.
use super::*;
use crate::loading::json_frames::RootFrames;
use std::{fs, io::Cursor};
fn facts(source: &[u8], ledger: &DecodeLedger) -> LiteralTextFacts {
    let mut count = 0;
    preflight::walk(Cursor::new(source), ledger, |_, _, _, _| {
        count += 1;
        Ok(())
    })
    .unwrap();
    let mut result = LiteralTextFacts::new(count, ledger).unwrap();
    preflight::walk(Cursor::new(source), ledger, |start, end, length, role| {
        result.record(start, end, length, role)?;
        Ok(())
    })
    .unwrap();
    result.finish().unwrap()
}
#[test]
fn nested_literals_preserve_duplicate_key_position_unicode_and_literal_reserved_keys() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for source in [
        br#"{"payload":{"x":["a",{"y":"\u00e9\ud83d\ude00"}],"x":"last","z":"kept"}}"#.as_slice(),
        br#"{"payload":{"$serde_json::private::RawValue":"not JSON","later":["",true,1]}}"#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("literal.json");
        fs::write(&path, source).unwrap();
        let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
        let baseline = ledger.live_requested_bytes();
        let mut held = HeldFile::open(&path).unwrap();
        let stamp = held.preflight_literal(&ledger).unwrap();
        let plan = held
            .plan_pass(&stamp)
            .scan(|reader| {
                let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap()
                else {
                    panic!()
                };
                frames.next_member().unwrap().unwrap()
            })
            .unwrap();
        let value =
            decode_authenticated_json(&plan, &mut held, |field| field.native_span(), &ledger)
                .unwrap()
                .unwrap();
        let value = value.as_owned().as_value();
        if value.get("x").is_some() {
            assert_eq!(value["x"], "last");
            assert_eq!(
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["x", "z"]
            );
        } else {
            assert_eq!(value["$serde_json::private::RawValue"], "not JSON");
            assert_eq!(value["later"][0], "");
        }
        assert_eq!(fs::read(&path).unwrap(), source);
        assert!(ledger.live_requested_bytes() > baseline);
    }
}
#[test]
fn lexical_facts_measure_all_duplicate_keys_and_decoded_unicode_bytes() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let source = br#"{"\u0078":"\u0061","x":"\ud83d\ude00"}"#;
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let plan = facts(source, &ledger);
    assert_eq!(plan.len(), 4);
    assert_eq!(plan.get(0).unwrap().decoded_bytes(), 1);
    assert_eq!(plan.get(1).unwrap().decoded_bytes(), 1);
    assert_eq!(plan.get(3).unwrap().decoded_bytes(), 4);
    for index in 0..plan.len() {
        let fact = plan.get(index).unwrap();
        assert_eq!(source[fact.start()], b'"');
        assert_eq!(source[fact.end() - 1], b'"');
    }
}
#[test]
fn current_nested_replay_rejects_changed_escape_layout_before_text_can_grow() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original = br#"{"x":["\u0061"]}"#;
    let changed = br#"{"x":["abcdef"]}"#;
    assert_eq!(original.len(), changed.len());
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let plan = facts(original, &ledger);
    let before = ledger.live_requested_bytes();
    let mut input = Input::new(Cursor::new(changed));
    let result = decode_current(&mut input, &ledger, Some(&plan), None);
    assert!(matches!(
        result,
        Err(LiteralReadError::Build(JsonDecodeError::TextLength {
            expected: 1,
            actual: 2
        }))
    ));
    assert_eq!(ledger.live_requested_bytes(), before);
}
#[test]
fn current_token_position_change_cannot_reuse_another_string_fact() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original = br#"["a","b"]"#;
    let changed = br#"["a", "b"]"#;
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let plan = facts(original, &ledger);
    let mut input = Input::new(Cursor::new(changed));
    assert!(matches!(
        decode_current(&mut input, &ledger, Some(&plan), None),
        Err(LiteralReadError::Build(JsonDecodeError::TextPlan))
    ));
}

#[test]
fn authenticated_lexical_frontier_counts_values_not_keys_and_keeps_duplicate_history_upper_bound() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    for (source, expected) in [
        (br#"{"first":"","first":"","first":""}"#.as_slice(), 3),
        (
            br#"{"first":[null,null,null],"other":null,"first":null}"#,
            3,
        ),
        (b"[]", 1),
        (b"{}", 1),
    ] {
        let (measured, frontier) =
            preflight::planned_walk(Cursor::new(source), &ledger, |_, _, _, _| Ok(())).unwrap();
        let original = preflight::preflight(Cursor::new(source), &ledger).unwrap();
        assert_eq!(frontier, expected);
        assert_eq!(
            (
                measured.nodes,
                measured.decoded_text_bytes,
                measured.structural_bytes
            ),
            (
                original.nodes,
                original.decoded_text_bytes,
                original.structural_bytes
            )
        );
    }
}
#[test]
fn real_planned_nested_root_uses_one_cleanup_slot_for_a_deep_single_child_chain() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deep.json");
    let depth = 4000;
    let source = format!(
        "{{\"payload\":{}null{}}}",
        "[".repeat(depth),
        "]".repeat(depth)
    );
    fs::write(&path, source).unwrap();
    let ledger = DecodeLedger::new(&authority, 16 * 1024 * 1024).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap() else {
                panic!()
            };
            frames.next_member().unwrap().unwrap()
        })
        .unwrap();
    let value = decode_authenticated_json(&plan, &mut held, |field| field.native_span(), &ledger)
        .unwrap()
        .unwrap();
    let alias = value.as_owned().clone();
    let receipt = alias.receipt().clone();
    let mut child = alias.as_value();
    for _ in 0..depth {
        child = &child.as_array().unwrap()[0];
    }
    assert!(child.is_null());
    drop(value);
    assert!(receipt.matches(&alias));
    std::thread::spawn(move || drop(alias)).join().unwrap();
    assert!(receipt.retained_identity_bytes().is_some());
}
