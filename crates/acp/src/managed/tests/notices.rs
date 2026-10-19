//! Source-owned notices preserve exact skip/default/presence projections.
use super::fixture;
use crate::managed::notice;
use agent_client_protocol::managed::Source;
use nocterm_ai::model::retention::RetentionAuthority;

#[test]
fn streamed_notices_preserve_presence_skips_and_original_session_identity() {
    for update in [
        r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}"#,
        r#"{"sessionUpdate":"user_message_chunk","content":{"type":"image","data":"all","mimeType":"image/png"}}"#,
        r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}"#,
        r#"{"sessionUpdate":"plan","entries":[{"content":"a","priority":"high","status":"pending"},false,{"content":"bad","priority":"future","status":"completed"},["b","low","completed"]]}"#,
        r#"{"sessionUpdate":"plan","entries":false}"#,
        r#"{"sessionUpdate":"plan"}"#,
        r#"{"sessionUpdate":"available_commands_update","availableCommands":[{"name":"go","description":"run","input":{"hint":"target"}},false,["stop","halt",["why"],null],{"name":"bad","description":9}]}"#,
        r#"{"sessionUpdate":"available_commands_update","availableCommands":null}"#,
        r#"{"sessionUpdate":"current_mode_update","currentModeId":"001"}"#,
        r#"{"sessionUpdate":"session_info_update"}"#,
        r#"{"sessionUpdate":"session_info_update","title":null}"#,
        r#"{"sessionUpdate":"session_info_update","title":42}"#,
        r#"{"sessionUpdate":"session_info_update","title":"new title"}"#,
    ] {
        compare(update);
    }
}

fn compare(update: &str) {
    let raw = format!(r#"{{"sessionId":"original-001","update":{update}}}"#);
    let fixture = fixture();
    let source = Source::copy(&raw, fixture.budget.clone()).unwrap();
    let native = notice::notice(&source.full(), &fixture.budget);
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let wire = serde_json::from_value::<agent_client_protocol_schema::v1::SessionNotification>(
        serde_json::from_str(&raw).unwrap(),
    );
    let original = wire
        .ok()
        .and_then(|wire| crate::mapping::notice::inbound(&authority, wire).ok())
        .flatten();
    match (native, original) {
        (Ok(Some(native)), Some(original)) => {
            assert_eq!(native.get(), &original, "source:{raw}")
        }
        (Err(_) | Ok(None), None) => {}
        (Ok(Some(_)), None) => panic!("native accepted original rejected:{raw}"),
        (Err(error), Some(_)) => panic!("native rejected original:{raw}: {error}"),
        (Ok(None), Some(_)) => panic!("native ignored original:{raw}"),
    }
}

#[test]
fn configuration_and_usage_preserve_untagged_defaults_and_rate_metadata() {
    for update in [
        r#"{"sessionUpdate":"config_option_update","configOptions":[{"id":"a","name":"A","type":"boolean","current_value":true}, {"id":"invalid","name":"I","type":"boolean","currentValue":true},false]}"#,
        r#"{"sessionUpdate":"config_option_update","configOptions":[{"id":"m","name":"Model","category":"future_category","description":42,"type":"select","currentValue":"1","options":[{"value":"1","name":"One","description":false,"_meta":{"a":[1,2]}}]}]}"#,
        r#"{"sessionUpdate":"config_option_update","configOptions":[{"id":"g","name":"Grouped","type":"select","currentValue":"a","options":[{"group":"g","name":"G","options":[{"value":"a","name":"A"},false,["b","B",null,null]]}]}]}"#,
        r#"{"sessionUpdate":"config_option_update","configOptions":[{"id":"bad","name":"Mixed","type":"select","currentValue":"a","options":[{"value":"a","name":"A"},{"group":"g","name":"G","options":[]}]}]}"#,
        r#"{"sessionUpdate":"config_option_update","configOptions":null}"#,
        r#"{"sessionUpdate":"usage_update","used":100,"size":200,"cost":{"amount":0.25,"currency":"USD"},"_meta":{"_claude/rateLimit":{"rateLimitType":"seven_day_opus","utilization":2,"resetsAt":123,"status":"rejected"}}}"#,
        r#"{"sessionUpdate":"usage_update","used":100,"size":200,"cost":[0.5,"EUR",null],"_meta":{"_claude/rateLimit":{"rateLimitType":"five_hour","utilization":-1}}}"#,
        r#"{"sessionUpdate":"usage_update","used":100,"size":200,"cost":{"amount":"bad","currency":"USD"},"_meta":{"_claude/rateLimit":{"rateLimitType":"unknown"}}}"#,
        r#"{"sessionUpdate":"usage_update","used":1.0,"size":200}"#,
        r#"{"sessionUpdate":"usage_update","used":-0,"size":200}"#,
    ] {
        compare(update);
    }
}

#[test]
fn tool_notices_preserve_original_claims_sparse_defaults_and_output_shapes() {
    for update in [
        r#"{"sessionUpdate":"tool_call","toolCallId":" 001 ","title":"title"}"#,
        r#"{"sessionUpdate":"tool_call","toolCallId":"id","title":"mcp_nocterm_17_read_terminal","name":"mcp.nocterm-17.read_terminal","kind":"execute","status":"in_progress","rawInput":{"server":"nocterm-17","tool":"read_terminal","arguments":{"tab_id":"001"}},"rawOutput":{"text":"done"},"_meta":{"nocterm/tool-display":{"spoof":"ignored"},"kept":1}}"#,
        r#"{"sessionUpdate":"tool_call","toolCallId":"id","title":"bad claim","name":"mcp.nocterm-17.unknown","kind":"future","status":"future","content":[false,{"type":"content","content":{"type":"text","text":"hello"}},{"type":"diff","path":"/tmp/a","newText":"new","oldText":7},{"type":"terminal","terminalId":"0001"},{"type":"future"}],"locations":[false,["/tmp/a",12,null],{"path":"/tmp/b","line":3.0}],"rawInput":null}"#,
        r#"{"sessionUpdate":"tool_call_update","toolCallId":"id"}"#,
        r#"{"sessionUpdate":"tool_call_update","toolCallId":"id","kind":"future","status":"completed","title":"changed","name":null,"content":[],"locations":[],"rawInput":{"server":"nocterm-1","tool":"read_terminal","arguments":{}},"rawOutput":null}"#,
        r#"{"sessionUpdate":"tool_call_update","toolCallId":"id","kind":true,"status":"future","title":42,"name":false,"content":false,"locations":null,"_meta":{"nocterm\/tool-display":{},"nocterm/tool-display":[1,2],"other":{"__raw_json":"ordinary"}}}"#,
    ] {
        compare(update);
    }
}
