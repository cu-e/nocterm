use super::content::{inbound, outbound};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::content::{Content, MediaKind};
use serde_json::json;
#[test]
fn every_supported_content_shape_preserves_supplied_facts() {
    let blocks = vec![
        json!({"type":"text","text":"literal","annotations":{"audience":["user","assistant"],"lastModified":"opaque","priority":0.5,"_meta":{"annotation":"kept"}},"_meta":{"vendor":"kept"}}),
        json!({"type":"image","data":"AA==","mimeType":"image/png","uri":"memory:001"}),
        json!({"type":"audio","data":"AA==","mimeType":"audio/wav"}),
        json!({"type":"resource_link","uri":"untrusted:file","name":"name","title":"title","description":"detail","mimeType":"text/plain","size":-1}),
        json!({"type":"resource","resource":{"uri":"untrusted:text","text":"opaque","mimeType":"text/plain","_meta":{"inner":true}},"_meta":{"outer":true}}),
        json!({"type":"resource","resource":{"uri":"untrusted:blob","blob":"AA==","mimeType":"application/binary"}}),
    ];
    for block in blocks {
        let original: wire::ContentBlock = serde_json::from_value(block).unwrap();
        let owned = inbound(original.clone()).unwrap();
        let stored = serde_json::to_vec(&owned).unwrap();
        let decoded: Content = serde_json::from_slice(&stored).unwrap();
        assert_eq!(outbound(decoded).unwrap(), original);
    }
}
#[test]
fn encoded_media_is_not_decoded_or_reencoded_by_mapping() {
    let original =
        wire::ContentBlock::Image(wire::ImageContent::new("not-base64", "provider/type"));
    let Content::Media(media) = inbound(original.clone()).unwrap() else {
        panic!("media expected")
    };
    assert_eq!(media.kind, MediaKind::Image);
    assert_eq!(media.encoded, "not-base64");
    assert_eq!(outbound(Content::Media(media)).unwrap(), original);
}

#[test]
fn sparse_tool_updates_keep_absent_values_and_clear_supplied_empty_lists() {
    let mut row = super::tools::record(
        wire::ToolCall::new("001", "title")
            .name("mcp.nocterm-001.read_terminal")
            .content(vec![wire::ToolCallContent::from("kept")]),
    )
    .unwrap();
    let null: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","name":null,"rawInput":null})).unwrap();
    assert!(row.apply(super::tools::patch(null).unwrap()));
    assert_eq!(row.name.as_deref(), Some("mcp.nocterm-001.read_terminal"));
    assert_eq!(row.output.len(), 1);
    let empty: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","content":[]})).unwrap();
    assert!(row.apply(super::tools::patch(empty).unwrap()));
    assert!(row.output.is_empty());
    assert_eq!(
        row.claims.consistent_identity(),
        Some(("001", "read_terminal"))
    );
}
#[test]
fn disagreeing_sparse_evidence_never_selects_a_bridge_destination() {
    let mut row = super::tools::record(
        wire::ToolCall::new("row", "display").name("mcp.nocterm-001.read_terminal"),
    )
    .unwrap();
    let update: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"row","title":"mcp__nocterm-1__read_terminal"}))
            .unwrap();
    row.apply(super::tools::patch(update).unwrap());
    assert_eq!(row.claims.consistent_identity(), None);
}
#[test]
fn forged_client_display_metadata_is_removed_at_live_ingress() {
    let row: wire::ToolCall = serde_json::from_value(json!({"toolCallId":"r","title":"title","_meta":{"nocterm/tool-display":{"destination":"forged"},"vendor":"inert"}})).unwrap();
    let row = super::tools::record(row).unwrap();
    let extensions = row.extensions.unwrap();
    assert!(!extensions.as_map().contains_key("nocterm/tool-display"));
    assert_eq!(extensions.as_map().get("vendor"), Some(&json!("inert")));
    assert_eq!(row.claims.consistent_identity(), None);
}

#[test]
fn configuration_groups_and_flags_remain_distinct_semantic_controls() {
    use nocterm_ai::model::controls::{ConfigKind, SelectChoices};
    let option: wire::SessionConfigOption = serde_json::from_value(json!({
        "id":"model","name":"Model","type":"select","currentValue":"001",
        "options":[{"group":"first","name":"First","options":[{"value":"001","name":"One"}]},
                   {"group":"second","name":"Second","options":[{"value":"1","name":"Another"}]}]
    }))
    .unwrap();
    let mapped = super::controls::option(option).unwrap();
    let ConfigKind::Select {
        current,
        choices: SelectChoices::Groups(groups),
    } = mapped.kind
    else {
        panic!("groups expected")
    };
    assert_eq!(current.as_str(), "001");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].choices[0].id.as_str(), "1");
    let flag: wire::SessionConfigOption = serde_json::from_value(
        json!({"id":"flag","name":"Flag","type":"boolean","currentValue":false}),
    )
    .unwrap();
    assert_eq!(
        super::controls::option(flag).unwrap().kind,
        ConfigKind::Boolean(false)
    );
}
