//! Provider identity cases cross the actual native mapper into own facts.
use agent_client_protocol_schema::v1 as acp;
use nocterm_ai::{
    TerminalCall,
    tool_display::{self, ToolDisplay, bridge_server_name, parse, request},
};
use serde_json::{Value, json};
fn mapped(call: &acp::ToolCall) -> nocterm_ai::model::tools::ToolRecord {
    super::super::tools::record(
        &nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap(),
        call.clone(),
    )
    .unwrap()
}
fn envelope(call: &acp::ToolCall, server: &str) -> Option<TerminalCall> {
    tool_display::envelope(&mapped(call), server)
}
fn raw_envelope(call: &acp::ToolCall, server: &str) -> Option<(String, Value)> {
    tool_display::raw_envelope(&mapped(call), server)
}
fn requested_call(call: &acp::ToolCall) -> Option<TerminalCall> {
    tool_display::requested_call(&mapped(call))
}
fn header(call: &acp::ToolCall) -> String {
    tool_display::header(&mapped(call))
}
fn command(server: &str) -> acp::ToolCall {
    acp::ToolCall::new("c1", format!("mcp.{server}.run_command")).raw_input(json!({"server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo hello"}}))
}
#[test]
fn envelope_requires_exact_server_and_validated_arguments() {
    let mut call = command("nocterm-3");
    assert!(envelope(&call, "nocterm-3").is_some());
    assert!(envelope(&call, "nocterm-30").is_none());
    call.raw_input.as_mut().unwrap()["arguments"]["command"] = json!("echo hello\necho forged");
    assert!(envelope(&call, "nocterm-3").is_none());
    call.raw_input = Some(json!({"arguments":{"command":"echo not nocterm"}}));
    assert!(envelope(&call, "nocterm-3").is_none());
    call.raw_input = Some(json!({"terminal_id":"t1","command":"echo direct"}));
    assert!(envelope(&call, "nocterm-3").is_some());
    call.name = Some("mcp.foreign.run_command".into());
    assert!(envelope(&call, "nocterm-3").is_none());
}

#[test]
fn fallback_never_guesses_a_host_or_relabels_external_tools() {
    let call = command("nocterm-3");
    assert!(header(&call).starts_with("Nocterm · Run command"));
    for title in [
        "Guardian Review",
        "mcp.external.run_command",
        "mcp.nocterm-forged.run_command",
    ] {
        let call = acp::ToolCall::new("c1", title);
        assert!(header(&call).starts_with(title));
    }
}

#[test]
fn a_foreign_tool_name_cannot_impersonate_a_nocterm_envelope() {
    let mut call = command("nocterm-3");
    call.name = Some("mcp.external.run_command".into());
    assert!(
        envelope(&call, "nocterm-3").is_none(),
        "a matching raw server field cannot override the explicit foreign tool identity",
    );
}

#[test]
fn a_foreign_title_or_mismatched_explicit_tool_cannot_impersonate_the_envelope() {
    let mut call = command("nocterm-3");
    call.title = "mcp.external.run_command".into();
    assert!(envelope(&call, "nocterm-3").is_none());
    call.name = Some("mcp.nocterm-3.send_input".into());
    assert!(envelope(&call, "nocterm-3").is_none());
    call.name = None;
    call.title = "Run command".into();
    assert!(envelope(&call, "nocterm-3").is_some());
}

#[test]
fn exact_provider_spellings_share_identity_and_validate_wrappers() {
    let input =
        json!({"terminal_id":"t1", "program":"bash", "args":["-lc","printf '%s\\n' 'hello'"]});
    let expected = request(&parse("exec_command", input.clone()).unwrap()).1;
    for title in [
        "mcp.nocterm-17.exec_command",
        "mcp__nocterm-17__exec_command",
    ] {
        for wrapped in [false, true] {
            let args = if wrapped {
                json!({"server":"nocterm-17", "tool":"exec_command", "arguments":input})
            } else {
                input.clone()
            };
            let call = acp::ToolCall::new("screenshot", title).raw_input(args);
            assert_eq!(request(&envelope(&call, "nocterm-17").unwrap()).1, expected);
            assert!(envelope(&call, "nocterm-1").is_none());
            assert!(header(&call).starts_with("Nocterm · Execute program"));
            assert!(ToolDisplay::from_call(&mapped(&call)).is_none());
        }
    }
    let mut call = acp::ToolCall::new("c1", "mcp__nocterm-17__exec_command").raw_input(input);
    call.name = Some("mcp.nocterm-17.exec_command".into());
    assert!(envelope(&call, "nocterm-17").is_some());
    call.name = Some("mcp__nocterm-170__exec_command".into());
    assert!(envelope(&call, "nocterm-17").is_none());
}

#[test]
fn malformed_and_foreign_doubleunderscore_calls_keep_the_provider_label() {
    for title in [
        "mcp__foreign__exec_command",
        "mcp__nocterm-x__exec_command",
        "mcp__nocterm-17__exec_command__extra",
        "mcp__nocterm-17__unknown",
        "mcp__nocterm-17_exec_command",
    ] {
        let call = acp::ToolCall::new("c1", title)
            .raw_input(json!({"terminal_id":"t1", "program":"true"}));
        assert!(requested_call(&call).is_none(), "{title}");
        assert!(envelope(&call, "nocterm-17").is_none());
        assert!(header(&call).starts_with(title));
    }
    for input in [
        json!({"server":"nocterm-170", "tool":"exec_command", "arguments":{"terminal_id":"t1", "program":"true"}}),
        json!({"server":"nocterm-17", "tool":"send_input", "arguments":{"terminal_id":"t1", "text":"foreign"}}),
        json!({"server":"nocterm-17", "arguments":{"terminal_id":"t1", "program":"true"}}),
    ] {
        let call = acp::ToolCall::new("c1", "mcp__nocterm-17__exec_command").raw_input(input);
        assert!(requested_call(&call).is_none());
        assert!(header(&call).starts_with("mcp__nocterm-17__exec_command"));
    }
}

#[test]
fn hermes_spelling_shares_identity_with_the_registered_server() {
    let input = json!({"terminal_id":"t2", "program":"sh", "args":["-c","qm config 9000"]});
    let call = acp::ToolCall::new("h1", "mcp_nocterm_3_exec_command").raw_input(input.clone());
    let expected = request(&parse("exec_command", input).unwrap());
    assert_eq!(request(&envelope(&call, "nocterm-3").unwrap()), expected);
    assert_eq!(request(&requested_call(&call).unwrap()), expected);
    assert!(envelope(&call, "nocterm-30").is_none());
    assert!(header(&call).starts_with("Nocterm · Execute program"));
    assert_eq!(bridge_server_name(3), "nocterm-3");
}

#[test]
fn hermes_omits_the_input_of_calls_without_arguments() {
    let call = acp::ToolCall::new("h1", "mcp_nocterm_12_list_terminals");
    assert!(matches!(
        envelope(&call, "nocterm-12"),
        Some(TerminalCall::ListTerminals)
    ));
    assert!(header(&call).starts_with("Nocterm · List terminals"));
    // A tool that needs arguments is not complete without them.
    let call = acp::ToolCall::new("h1", "mcp_nocterm_12_run_command");
    assert!(envelope(&call, "nocterm-12").is_none());
    assert!(header(&call).starts_with("Nocterm · Run command"));
}

#[test]
fn malformed_and_foreign_hermes_names_keep_the_provider_label() {
    for title in [
        "mcp_nocterm_x_run_command",
        "mcp_nocterm_3run_command",
        "mcp_nocterm_3_unknown",
        "mcp_github_run_command",
        "mcp_nocterm_3_exec_command_extra",
    ] {
        let call = acp::ToolCall::new("h1", title)
            .raw_input(json!({"terminal_id":"t1", "command":"true"}));
        assert!(requested_call(&call).is_none(), "{title}");
        assert!(envelope(&call, "nocterm-3").is_none(), "{title}");
        assert!(header(&call).starts_with(title), "{title}");
    }
    let mut call = acp::ToolCall::new("h1", "mcp_nocterm_3_run_command")
        .raw_input(json!({"terminal_id":"t1", "command":"true"}));
    call.name = Some("mcp__nocterm-4__run_command".into());
    assert!(envelope(&call, "nocterm-3").is_none());
    assert!(envelope(&call, "nocterm-4").is_none());
}

#[test]
fn a_request_the_bridge_rejected_is_still_shown_but_never_matched() {
    // From a Codex chat: a build with a timeout past the bridge's limit.
    let arguments = json!({"terminal_id":"t1", "program":"/bin/sh",
        "args":["-c","cargo build --release"], "timeout_ms":1_800_000, "yield_ms":1000});
    for input in [
        json!({"server":"nocterm-11", "tool":"exec_command", "arguments":arguments}),
        json!({"terminal_id":"t1", "program":""}),
    ] {
        let call = acp::ToolCall::new("c1", "mcp.nocterm-11.exec_command").raw_input(input);
        assert!(requested_call(&call).unwrap().validate().is_err());
        assert!(header(&call).starts_with("Nocterm · Execute program"));
        assert!(
            envelope(&call, "nocterm-11").is_none(),
            "only requests the bridge would run are matched to its records"
        );
    }
}

#[test]
fn raw_matching_preserves_rejected_unknown_tools_and_original_arguments() {
    for title in [
        "mcp.nocterm-3.typo",
        "mcp__nocterm-3__typo",
        "mcp_nocterm_3_typo",
    ] {
        let call = acp::ToolCall::new("bad", title).raw_input(json!({"wrong":true}));
        assert_eq!(
            raw_envelope(&call, "nocterm-3"),
            Some(("typo".into(), json!({"wrong":true})))
        );
        assert!(raw_envelope(&call, "nocterm-4").is_none());
        assert!(envelope(&call, "nocterm-3").is_none());
    }
}

#[test]
fn old_hermes_server_names_are_display_only() {
    for tool in ["exec_command", "list_terminals"] {
        let input = if tool == "exec_command" {
            json!({"terminal_id":"t1","program":"sh"})
        } else {
            json!({})
        };
        let call = acp::ToolCall::new("old", format!("mcp_nocterm_{tool}")).raw_input(input);
        assert!(requested_call(&call).is_some());
        assert!(header(&call).starts_with("Nocterm"));
        assert!(envelope(&call, "nocterm-3").is_none());
        assert!(raw_envelope(&call, "nocterm-3").is_none());
    }
    let mut call = acp::ToolCall::new("old", "mcp_nocterm_exec_command")
        .raw_input(json!({"terminal_id":"t1","program":"sh"}));
    call.name = Some("mcp.nocterm-3.exec_command".into());
    assert!(requested_call(&call).is_none());
    assert!(envelope(&call, "nocterm-3").is_none());
}

#[test]
fn literal_ranges_keep_unknown_and_long_registration_without_authority_copies() {
    let owner = "0".repeat(100_000);
    let title = format!("mcp.nocterm-{owner}.typo");
    let call =
        acp::ToolCall::new("literal id", title.clone()).raw_input(json!({"wrong":"literal"}));
    let own = mapped(&call);
    assert_eq!(own.title, title);
    assert!(matches!(
        own.claims.title,
        nocterm_ai::model::tools::Claim::Invalid
    ));
    assert_eq!(
        tool_display::raw_envelope(&own, &format!("nocterm-{owner}")),
        Some(("typo".into(), json!({"wrong":"literal"})))
    );
    assert!(tool_display::envelope(&own, "nocterm-0").is_none());
    assert!(tool_display::requested_call(&own).is_none());
    assert_eq!(tool_display::header(&own), format!("{title} · Pending"));
    let serialized = serde_json::to_value(&own).unwrap();
    assert!(serialized.get("claims").is_none());
    assert!(serialized.get("literal").is_none());
}

#[test]
fn shape_parsing_and_original_identity_do_not_renew_execution_validation() {
    let arguments = json!({"terminal_id":"t1","lines":2001});
    let row = acp::ToolCall::new("row", "mcp.nocterm-7.read_terminal").raw_input(arguments.clone());
    assert!(requested_call(&row).is_some());
    assert!(envelope(&row, "nocterm-7").is_none());
    let record = mapped(&row);
    assert_eq!(
        record.arguments.as_ref().map(|value| value.as_value()),
        Some(&arguments)
    );
    assert!(record.claims.consistent_identity().is_some());
}
