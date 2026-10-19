//! The neutral records read and write exactly the JSON the protocol records do.
use super::{
    content::{content, wire_content},
    controls::option,
    tools::{patch, record},
};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    content::Content,
    controls::{ConfigChoices, ConfigKind},
    tools::{ToolKind, ToolRecord, ToolStatus},
};
use serde_json::{Value, json};

fn content_corpus() -> Vec<Value> {
    vec![
        json!({"type":"text","text":"literal","annotations":{"audience":["user","assistant"],"lastModified":"opaque","priority":0.5,"_meta":{"annotation":"kept"}},"_meta":{"vendor":"kept","zeta":1,"alpha":2}}),
        json!({"type":"image","data":"AA==","mimeType":"image/png","uri":"memory:001"}),
        json!({"type":"audio","data":"AA==","mimeType":"audio/wav","annotations":{}}),
        json!({"type":"resource_link","uri":"untrusted:file","name":"name","title":"title","description":"detail","mimeType":"text/plain","size":-1}),
        json!({"type":"resource","resource":{"uri":"untrusted:text","text":"opaque","mimeType":"text/plain","_meta":{"inner":true}},"_meta":{"outer":true}}),
        json!({"type":"resource","resource":{"uri":"untrusted:blob","blob":"AA==","mimeType":"application/binary"}}),
    ]
}
fn malformed_content() -> Vec<Value> {
    vec![
        json!({"type":"text"}),
        json!({"type":"unknown","text":"x"}),
        json!({"text":"untyped"}),
        json!({"type":"image","data":1,"mimeType":"image/png"}),
        json!({"type":"text","text":"x","annotations":{"priority":"high"},"_meta":5}),
        json!({"type":"text","text":"x","annotations":{"audience":["user","robot",3,"assistant"]}}),
        json!({"type":"text","text":"x","annotations":"none"}),
        json!({"type":"resource_link","name":"n","uri":"u","size":"big","title":7}),
        json!({"type":"resource","resource":{"uri":"u"}}),
        json!({"type":"resource","resource":{"uri":"u","text":"t","blob":"b"}}),
        json!({"type":"image","data":"AA==","mimeType":"image/png","uri":null,"_meta":null}),
    ]
}
fn tool_corpus() -> Vec<Value> {
    vec![
        json!({"toolCallId":"001","title":"minimal"}),
        json!({"toolCallId":"002","title":"full","name":"mcp.nocterm-001.read_terminal","kind":"switch_mode","status":"in_progress",
            "content":[{"type":"content","content":{"type":"text","text":"out"},"_meta":{"item":1}},
                       {"type":"diff","path":"/tmp/a","oldText":"a","newText":"b"},
                       {"type":"terminal","terminalId":"t1"}],
            "locations":[{"path":"/tmp/a","line":3,"_meta":{"loc":1}}],
            "rawInput":{"command":"ls"},"rawOutput":{"exit":0},"_meta":{"tool":"meta"}}),
        json!({"toolCallId":"003","title":"done","kind":"read","status":"completed"}),
        json!({"toolCallId":"004","title":"failed","kind":"execute","status":"failed","rawInput":0,"rawOutput":false}),
    ]
}
fn malformed_tools() -> Vec<Value> {
    vec![
        json!({"title":"no id"}),
        json!({"toolCallId":"1"}),
        json!({"toolCallId":"1","title":"t","kind":"mystery","status":"mystery"}),
        json!({"toolCallId":"1","title":"t","kind":5,"status":[]}),
        json!({"toolCallId":"1","title":"t","rawInput":null,"rawOutput":null,"name":null}),
        json!({"toolCallId":"1","title":"t","content":[{"type":"bogus"},{"type":"content","content":{"type":"text","text":"ok"}},{"type":"content"}]}),
        json!({"toolCallId":"1","title":"t","content":"nope","locations":"nope"}),
        json!({"toolCallId":"1","title":"t","locations":[{"line":1},{"path":"/ok","line":"x"},{"path":"/ok2"}]}),
        json!({"toolCallId":"1","title":"t","_meta":"bad"}),
        json!({"toolCallId":"1","title":"t","content":[],"locations":[],"kind":"other","status":"pending"}),
    ]
}
macro_rules! bytes {
    ($value:expr) => {
        serde_json::to_vec(&$value).unwrap()
    };
}

#[test]
fn content_writes_the_same_bytes_as_the_protocol_record() {
    for json in content_corpus() {
        let sdk: wire::ContentBlock = serde_json::from_value(json).unwrap();
        let neutral = content(sdk.clone()).unwrap();
        assert_eq!(bytes!(neutral), bytes!(sdk));
        assert_eq!(wire_content(neutral), sdk);
    }
}
#[test]
fn content_reads_the_same_records_as_the_protocol() {
    for json in content_corpus().into_iter().chain(malformed_content()) {
        let sdk = serde_json::from_value::<wire::ContentBlock>(json.clone());
        let neutral = serde_json::from_value::<Content>(json.clone());
        match (sdk, neutral) {
            (Ok(sdk), Ok(neutral)) => {
                assert_eq!(content(sdk.clone()).unwrap(), neutral, "{json}");
                assert_eq!(bytes!(neutral), bytes!(sdk), "{json}");
            }
            (Err(_), Err(_)) => {}
            (sdk, neutral) => panic!("decode disagreement for {json}: {sdk:?} / {neutral:?}"),
        }
    }
}
#[test]
fn tool_rows_write_the_same_bytes_as_the_protocol_record() {
    for json in tool_corpus() {
        let sdk: wire::ToolCall = serde_json::from_value(json).unwrap();
        let neutral = record(sdk.clone()).unwrap();
        assert_eq!(bytes!(neutral), bytes!(sdk));
    }
}
#[test]
fn tool_rows_read_the_same_records_as_the_protocol() {
    for json in tool_corpus().into_iter().chain(malformed_tools()) {
        let sdk = serde_json::from_value::<wire::ToolCall>(json.clone());
        let neutral = serde_json::from_value::<ToolRecord>(json.clone());
        match (sdk, neutral) {
            (Ok(sdk), Ok(neutral)) => {
                assert_eq!(record(sdk.clone()).unwrap(), neutral, "{json}");
                assert_eq!(bytes!(neutral), bytes!(sdk), "{json}");
            }
            (Err(_), Err(_)) => {}
            (sdk, neutral) => panic!("decode disagreement for {json}: {sdk:?} / {neutral:?}"),
        }
    }
}
#[test]
fn an_unrecognized_kind_is_other_and_status_defaults_to_pending() {
    let neutral: ToolRecord =
        serde_json::from_value(json!({"toolCallId":"1","title":"t","kind":"mystery","status":9}))
            .unwrap();
    assert_eq!(neutral.kind, ToolKind::Other);
    assert_eq!(neutral.status, ToolStatus::Pending);
}

#[test]
fn sparse_tool_updates_keep_absent_values_and_clear_supplied_empty_lists() {
    let mut row = record(
        wire::ToolCall::new("001", "title")
            .name("mcp.nocterm-001.read_terminal")
            .content(vec![wire::ToolCallContent::from("kept")]),
    )
    .unwrap();
    let null: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","name":null,"rawInput":null})).unwrap();
    row.apply(patch(null).unwrap().changes);
    assert_eq!(row.name.as_deref(), Some("mcp.nocterm-001.read_terminal"));
    assert_eq!(row.content.len(), 1);
    assert!(row.raw_input.is_none());
    let empty: wire::ToolCallUpdate =
        serde_json::from_value(json!({"toolCallId":"001","content":[]})).unwrap();
    row.apply(patch(empty).unwrap().changes);
    assert!(row.content.is_empty());
}

#[test]
fn configuration_groups_and_flags_remain_distinct_controls() {
    let grouped: wire::SessionConfigOption = serde_json::from_value(json!({
        "id":"model","name":"Model","type":"select","currentValue":"001",
        "options":[{"group":"first","name":"First","options":[{"value":"001","name":"One"}]},
                   {"group":"second","name":"Second","options":[{"value":"1","name":"Another"}]}]
    }))
    .unwrap();
    let mapped = option(grouped).unwrap();
    let ConfigKind::Select(select) = mapped.kind else {
        panic!("select expected")
    };
    assert_eq!(select.current.as_str(), "001");
    let ConfigChoices::Grouped(groups) = select.choices else {
        panic!("groups expected")
    };
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].choices[0].id.as_str(), "1");
    let flag: wire::SessionConfigOption = serde_json::from_value(
        json!({"id":"flag","name":"Flag","type":"boolean","currentValue":false}),
    )
    .unwrap();
    assert_eq!(option(flag).unwrap().kind, ConfigKind::Boolean(false));
}
