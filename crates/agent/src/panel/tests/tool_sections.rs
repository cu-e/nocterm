use super::tool_output::exec_result;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::{acp, tool_display};
use serde_json::json;

#[test]
fn malformed_input_does_not_make_the_title_or_output_raw() {
    for title in [
        "mcp.nocterm-3.exec_command",
        "mcp__nocterm-3__exec_command",
        "mcp_nocterm_3_exec_command",
        "mcp_nocterm_exec_command",
    ] {
        let call = acp::ToolCall::new("malformed",title).raw_input(json!({"program":7,"args":["literal"]}))
            .raw_output(json!({"result":{"content":[{"type":"text","text":exec_result("exited").to_string()}],"_meta":{}},"id":7,"jsonrpc":"2.0"}));
        assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
        let input = tool_input::source(&call).unwrap();
        assert_eq!(input.label, "Requested parameters");
        assert_eq!(input.language, None);
        assert!(input.text.contains("program: 7"));
        assert!(!input.text.starts_with('{'));
        assert_eq!(
            tool_output::source(&call, false).unwrap().sections[0].label,
            "Standard output"
        );
    }
}

#[test]
fn malformed_bridge_arguments_remain_frozen_and_have_readable_error_output() {
    let mut call = acp::ToolCall::new("rejected", "mcp.nocterm-3.exec_command")
        .raw_input(json!({"program":"forged"}))
        .raw_output(json!({"provider":"wrong"}));
    let mut display = tool_display::ToolDisplay::requested(
        "exec_command".into(),
        json!({"program":false,"extra":{"field":"literal"}}),
    );
    display.finish(Err("Invalid arguments".into()));
    display.attach(&mut call);
    assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
    let input = tool_input::source(&call).unwrap();
    assert!(input.text.contains("program: false"));
    assert!(!input.text.contains("forged"));
    let output = tool_output::source(&call, false).unwrap();
    assert_eq!(output.sections[0].label, "Error");
    assert_eq!(output.sections[0].text, "Invalid arguments");
}

#[test]
fn unknown_outputs_do_not_replace_known_titles_and_requests() {
    let call = acp::ToolCall::new("future","mcp.nocterm-3.exec_command")
        .raw_input(json!({"terminal_id":"t1","program":"sh","args":["-c","echo literal"]}))
        .raw_output(json!({"content":[{"type":"text","text":"{\"future_output\":\"still visible\"}"}],"_meta":{}}));
    assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
    assert_eq!(tool_input::source(&call).unwrap().text, "echo literal");
    assert_eq!(
        tool_output::source(&call, false).unwrap().sections[0].text,
        "future_output: still visible"
    );
}

#[test]
fn explicit_registered_envelopes_work_without_a_provider_name() {
    let call = acp::ToolCall::new("anonymous", "Tool call")
        .raw_input(
            json!({"server":"nocterm-3","tool":"exec_command","arguments":{"program":false}}),
        )
        .raw_output(json!({"result":exec_result("exited")}));
    assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
    assert_eq!(tool_input::source(&call).unwrap().text, "program: false");
    assert_eq!(
        tool_output::source(&call, false).unwrap().sections[0].label,
        "Standard output"
    );
    let mut foreign = call;
    foreign.name = Some("mcp.external.exec_command".into());
    assert!(tool_display::requested_arguments(&foreign).is_none());
    assert!(tool_display::header(&foreign).starts_with("Tool call"));
}
