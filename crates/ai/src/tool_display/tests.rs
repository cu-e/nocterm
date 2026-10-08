use super::*;

fn command(server: &str) -> acp::ToolCall {
    acp::ToolCall::new("c1", format!("mcp.{server}.run_command")).raw_input(json!({
        "server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo hello"}
    }))
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
fn local_metadata_survives_history_and_provider_metadata_is_removed() {
    let mut call = command("nocterm-3");
    let request = envelope(&call, "nocterm-3").unwrap();
    let display = ToolDisplay::new(&request, Some("Production · root@actual.example:22".into()));
    display.attach(&mut call);
    let restored: acp::ToolCall =
        serde_json::from_value(serde_json::to_value(&call).unwrap()).unwrap();
    assert!(header(&restored).starts_with("Production · root@actual.example:22 · Run command"));
    let mut incoming = acp::SessionUpdate::ToolCall(restored);
    strip_provider_meta(&mut incoming);
    let acp::SessionUpdate::ToolCall(incoming) = incoming else {
        unreachable!()
    };
    assert!(ToolDisplay::from_call(&incoming).is_none());
    assert!(header(&incoming).starts_with("Nocterm · Run command"));
    assert!(!header(&incoming).contains("actual.example"));
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
fn redaction_expansion_keeps_a_verified_display_without_weakening_execution_validation() {
    let command = "password=1 ".repeat(1400);
    assert!(command.len() < crate::MAX_INPUT_BYTES);
    let request = parse("run_command", json!({"terminal_id":"t1","command":command})).unwrap();
    let mut display = ToolDisplay::new(&request, Some("Original host".into()));
    display.redact();
    assert!(display.arguments["command"].as_str().unwrap().len() > crate::MAX_INPUT_BYTES);
    assert!(parse("run_command", display.arguments.clone()).is_none());
    let mut call = acp::ToolCall::new("c1", "provider title");
    display.attach(&mut call);
    let saved = ToolDisplay::from_call(&call).unwrap();
    assert!(saved.redacted);
    assert!(saved.title().starts_with("Original host"));
    assert!(saved.display_request().is_some());
    assert!(!saved.arguments.to_string().contains("password=1"));
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
            assert!(ToolDisplay::from_call(&call).is_none());
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
        json!({"terminal_id":"t1", "program":""}),
    ] {
        let call = acp::ToolCall::new("c1", "mcp__nocterm-17__exec_command").raw_input(input);
        assert!(requested_call(&call).is_none());
        assert!(header(&call).starts_with("mcp__nocterm-17__exec_command"));
    }
}
