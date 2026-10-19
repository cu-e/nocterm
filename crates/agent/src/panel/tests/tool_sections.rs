use super::tool_output::exec_result;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::{
    model::{
        json::OwnedJson,
        literal::{LiteralIdentity, TextRange},
        retention::RetentionAuthority,
        tools::ToolStatus,
    },
    tool_display,
};
use serde_json::json;

#[test]
fn malformed_input_does_not_make_the_title_or_output_raw() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for (title, owner, tool) in [
        ("mcp.nocterm-3.exec_command", Some(12..13), 14..26),
        ("mcp__nocterm-3__exec_command", Some(13..14), 16..28),
        ("mcp_nocterm_3_exec_command", Some(12..13), 14..26),
        ("mcp_nocterm_exec_command", None, 12..24),
    ] {
        let mut call =
            super::provider_fixtures::provider_tool("malformed", title, ToolStatus::Pending);
        let tool = TextRange::supplied(&call.title, &call.title[tool]).unwrap();
        call.literal.title = match owner {
            Some(owner) => LiteralIdentity::Registered {
                owner: TextRange::supplied(&call.title, &call.title[owner]).unwrap(),
                tool,
            },
            None => LiteralIdentity::Historical { tool },
        };
        call.arguments = Some(
            OwnedJson::from_untracked(&authority, json!({"program":7,"args":["literal"]})).unwrap(),
        );
        call.result = Some(OwnedJson::from_untracked(&authority,json!({"result":{"content":[{"type":"text","text":exec_result("exited").to_string()}],"_meta":{}},"id":7,"jsonrpc":"2.0"})).unwrap());
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
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = super::provider_fixtures::provider_tool(
        "rejected",
        "mcp.nocterm-3.exec_command",
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_fixtures::known_title(&mut call, 12..13, 14..end);
    call.arguments =
        Some(OwnedJson::from_untracked(&authority, json!({"program":"forged"})).unwrap());
    call.result = Some(OwnedJson::from_untracked(&authority, json!({"provider":"wrong"})).unwrap());
    let mut display = tool_display::ToolDisplay::requested(
        &authority,
        "exec_command".into(),
        json!({"program":false,"extra":{"field":"literal"}}),
    )
    .unwrap();
    display.finish(Err("Invalid arguments".into())).unwrap();
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
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = super::provider_fixtures::provider_tool(
        "future",
        "mcp.nocterm-3.exec_command",
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_fixtures::known_title(&mut call, 12..13, 14..end);
    call.arguments = Some(
        OwnedJson::from_untracked(
            &authority,
            json!({"terminal_id":"t1","program":"sh","args":["-c","echo literal"]}),
        )
        .unwrap(),
    );
    call.result=Some(OwnedJson::from_untracked(&authority,json!({"content":[{"type":"text","text":"{\"future_output\":\"still visible\"}"}],"_meta":{}})).unwrap());
    assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
    assert_eq!(tool_input::source(&call).unwrap().text, "echo literal");
    assert_eq!(
        tool_output::source(&call, false).unwrap().sections[0].text,
        "future_output: still visible"
    );
}

#[test]
fn explicit_registered_envelopes_work_without_a_provider_name() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call =
        super::provider_fixtures::provider_tool("anonymous", "Tool call", ToolStatus::Pending);
    call.arguments = Some(
        OwnedJson::from_untracked(
            &authority,
            json!({"server":"nocterm-3","tool":"exec_command","arguments":{"program":false}}),
        )
        .unwrap(),
    );
    let server = call.arguments.as_ref().unwrap()["server"].as_str().unwrap();
    let tool = call.arguments.as_ref().unwrap()["tool"].as_str().unwrap();
    call.literal.input = LiteralIdentity::Registered {
        owner: TextRange::supplied(server, &server[8..]).unwrap(),
        tool: TextRange::supplied(tool, tool).unwrap(),
    };
    call.result = Some(
        OwnedJson::from_untracked(&authority, json!({"result":exec_result("exited")})).unwrap(),
    );
    assert!(tool_display::header(&call).starts_with("Nocterm · Execute program"));
    assert_eq!(tool_input::source(&call).unwrap().text, "program: false");
    assert_eq!(
        tool_output::source(&call, false).unwrap().sections[0].label,
        "Standard output"
    );
    let mut foreign = call;
    foreign.name = Some("mcp.external.exec_command".into());
    foreign.literal.name = LiteralIdentity::Invalid;
    assert!(tool_display::requested_arguments(&foreign).is_none());
    assert!(tool_display::header(&foreign).starts_with("Tool call"));
}
