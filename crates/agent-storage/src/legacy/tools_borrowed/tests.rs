use super::*;
use agent_client_protocol_schema::v1 as wire;
use serde_json::json;
#[test]
fn borrowed_tool_matches_private_sdk_bytes_without_payload_clone() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let fixtures = [
        json!({"toolCallId":"original","title":"original title"}),
        json!({"toolCallId":"original","title":"old","name":"literal","kind":"switch_mode","status":"in_progress","rawInput":{"value":null},"rawOutput":{"keep":"output"},"content":[{"type":"content","content":{"type":"image","data":"full","mimeType":"image/png"}},{"type":"diff","path":"/original","oldText":"old","newText":"new"},{"type":"terminal","terminalId":"old-receipt"}],"locations":[{"path":"/original","line":4,"_meta":{"x":true}}],"_meta":{"aaa":null,"zzz":false,"nocterm/tool-display":{"version":2,"tool":"old","arguments":{},"destination":null,"outcome":{"Ok":null}}}}),
        json!({"toolCallId":"id","title":"old","_meta":{"nocterm/tool-display":{"version":2,"tool":"old","arguments":{},"future":null}}}),
    ];
    for fixture in fixtures {
        let native: wire::ToolCall = serde_json::from_value(fixture).unwrap();
        let expected = serde_json::to_vec(&native).unwrap();
        let private: super::super::tools_decode::Call = serde_json::from_slice(&expected).unwrap();
        let (owned, order) = super::super::tools::moving_ordered(&authority, private).unwrap();
        check(&owned).unwrap();
        let actual = serde_json::to_vec(&Borrowed(&owned, order.as_ref())).unwrap();
        assert_eq!(actual, expected);
        let measured =
            nocterm_ai::history::measure_json(&Borrowed(&owned, order.as_ref())).unwrap();
        assert_eq!(measured.encoded_bytes, actual.len());
        assert_eq!(
            measured.structural_bytes,
            nocterm_ai::history::structural_bytes(&actual).unwrap()
        );
    }
}

fn imported(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    meta: serde_json::Value,
) -> (ToolRecord, Option<super::super::order::RowOrder>) {
    let native: wire::ToolCall =
        serde_json::from_value(json!({"toolCallId":"","title":"same","_meta":meta})).unwrap();
    let private = serde_json::from_slice(&serde_json::to_vec(&native).unwrap()).unwrap();
    super::super::tools::moving_ordered(authority, private).unwrap()
}
fn metadata_bytes(row: &ToolRecord, order: Option<&super::super::order::RowOrder>) -> Vec<u8> {
    serde_json::to_vec(&Metadata(row, order)).unwrap()
}
#[test]
fn original_outer_slots_and_every_display_key_permutation_replay_exact_sdk_bytes() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let fields = [
        "version",
        "tool",
        "arguments",
        "destination",
        "redacted",
        "outcome",
    ];
    fn permutations(fields: &mut [&str], index: usize, visit: &mut impl FnMut(&[&str])) {
        if index == fields.len() {
            visit(fields);
            return;
        }
        for next in index..fields.len() {
            fields.swap(index, next);
            permutations(fields, index + 1, visit);
            fields.swap(index, next);
        }
    }
    permutations(&mut fields.clone(), 0, &mut |keys| {
        let values = json!({"version":2,"tool":"historical","arguments":{"z":1,"a":2},"destination":null,"redacted":false,"outcome":{"Ok":{"z":null,"a":false}}});
        let display = serde_json::Value::Object(
            keys.iter()
                .map(|key| ((*key).into(), values[*key].clone()))
                .collect(),
        );
        for slot in 0..3 {
            let mut meta = serde_json::Map::new();
            for index in 0..3 {
                if index == slot {
                    meta.insert(DISPLAY_KEY.into(), display.clone());
                } else {
                    meta.insert(format!("vendor{index}"), json!(index));
                }
            }
            let expected = serde_json::to_vec(&meta).unwrap();
            let (row, order) = imported(&authority, serde_json::Value::Object(meta));
            assert_eq!(metadata_bytes(&row, order.as_ref()), expected);
        }
    });
}
#[test]
fn clone_reorder_same_id_replacement_and_metadata_replacement_use_original_ownership() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let original_meta = json!({"before":1,"nocterm/tool-display":{"arguments":{},"tool":"old","version":2},"after":2});
    let (original, order) = imported(&authority, original_meta.clone());
    let mut grammar = super::super::borrowed::Grammar::new(&authority);
    grammar.record(order).unwrap();
    let (other, order) = imported(
        &authority,
        json!({"nocterm/tool-display":{"version":2,"tool":"second","arguments":{}},"after":2}),
    );
    grammar.record(order).unwrap();
    let mut rows = [original.clone(), other, original.clone()];
    rows.swap(0, 1);
    for row in [&rows[1], &rows[2]] {
        assert_eq!(
            metadata_bytes(row, grammar.order(row)),
            serde_json::to_vec(&original_meta).unwrap()
        );
    }
    let mut replacement = original.clone();
    replacement.id = nocterm_ai::model::ids::ToolId::new("");
    assert!(grammar.order(&replacement).is_none());
    assert_eq!(metadata_bytes(&replacement, None), serde_json::to_vec(&json!({"before":1,"after":2,"nocterm/tool-display":{"version":2,"tool":"old","arguments":{}}})).unwrap());
    replacement = original.clone();
    replacement.extensions = Some(
        nocterm_ai::model::extensions::Extensions::checked(
            &authority,
            serde_json::from_value(json!({"after":2,"before":1})).unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(metadata_bytes(&replacement, grammar.order(&replacement)), serde_json::to_vec(&json!({"after":2,"before":1,"nocterm/tool-display":{"arguments":{},"tool":"old","version":2}})).unwrap());
}
#[test]
fn equal_display_attachment_keeps_provenance_changed_attachment_uses_native_inner_order() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let meta = json!({"before":1,"nocterm/tool-display":{"arguments":{},"tool":"old","version":2},"after":2});
    let (mut row, order) = imported(&authority, meta.clone());
    let mut display = row.client_display.as_deref().unwrap().clone();
    display.attach(&mut row);
    assert_eq!(
        metadata_bytes(&row, order.as_ref()),
        serde_json::to_vec(&meta).unwrap()
    );
    display.tool = "new".into();
    display.attach(&mut row);
    assert_eq!(metadata_bytes(&row, order.as_ref()), serde_json::to_vec(&json!({"before":1,"nocterm/tool-display":{"version":2,"tool":"new","arguments":{}},"after":2})).unwrap());
}
