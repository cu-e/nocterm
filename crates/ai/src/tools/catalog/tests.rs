use super::*;
use crate::{
    acp,
    mcp::*,
    tool_display::{self, ToolDisplay, ToolOutcome},
};
use std::collections::BTreeSet;

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("../fixtures/calls.json")).unwrap()
}

fn session() -> McpSession {
    let mut session = McpSession::default();
    session.handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
    session
}

fn invoke(session: &mut McpSession, name: &str, arguments: &Value, id: Value) -> McpStep {
    session.handle(
        &json!({"jsonrpc":"2.0","id":id,"method":"tools/call", "params":{
            "name":name,"arguments":arguments
        }})
        .to_string(),
    )
}

fn correct_variant(name: &str, call: &TerminalCall) -> bool {
    matches!(
        (name, call),
        ("list_terminals", TerminalCall::ListTerminals)
            | ("open_terminal", TerminalCall::OpenTerminal(_))
            | ("read_terminal", TerminalCall::ReadTerminal(_))
            | ("send_input", TerminalCall::SendInput(_))
            | ("run_command", TerminalCall::RunCommand(_))
            | ("exec_command", TerminalCall::ExecCommand(_))
            | ("read_command", TerminalCall::ReadCommand(_))
            | ("cancel_command", TerminalCall::CancelCommand(_))
    )
}

#[test]
fn tools_list_preserves_original_order_descriptions_and_schemas() {
    let expected: Value =
        serde_json::from_str(include_str!("../fixtures/tools-list.json")).unwrap();
    let McpStep::Reply(actual) =
        session().handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
    else {
        panic!("expected tools/list reply");
    };
    assert_eq!(actual, expected);
}

#[test]
fn every_variant_round_trips_through_a_unique_catalogue_entry() {
    let fixtures = fixtures();
    let names: BTreeSet<_> = TOOLS.iter().map(|spec| spec.name).collect();
    assert_eq!(TOOLS.len(), 8);
    assert_eq!(TOOLS.len(), names.len());
    assert_eq!(fixtures.len(), names.len());
    assert_eq!(
        fixtures
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        names
    );
    for fixture in fixtures {
        let name = fixture["name"].as_str().unwrap();
        let spec = get(name).unwrap();
        let call = (spec.parse)(fixture["arguments"].clone()).unwrap();
        assert!(correct_variant(name, &call));
        assert_eq!(call.name(), name);
        assert_eq!(spec.label, fixture["label"]);
        assert_eq!(tool_display::operation(name), spec.label);
        assert!(call.validate().is_ok());
        assert_eq!(
            tool_display::request(&call),
            (spec.name, fixture["normalized"].clone())
        );
        let restored = (spec.parse)(tool_display::request(&call).1).unwrap();
        assert!(correct_variant(name, &restored));
        assert_eq!(tool_display::request(&restored).1, fixture["normalized"]);
    }
    assert!(get("unknown").is_none());
    assert_eq!(tool_display::operation("unknown"), "Tool");
}

#[test]
fn mcp_preserves_original_arguments_ids_and_optional_defaults_for_all_tools() {
    let mut session = session();
    for fixture in fixtures() {
        let name = fixture["name"].as_str().unwrap();
        for expected_id in [json!(41), json!("request"), Value::Null] {
            let McpStep::Call {
                id,
                call,
                arguments,
            } = invoke(
                &mut session,
                name,
                &fixture["arguments"],
                expected_id.clone(),
            )
            else {
                panic!("expected call for {name}");
            };
            assert_eq!(id, expected_id);
            assert_eq!(arguments, fixture["arguments"]);
            assert!(correct_variant(name, &call));
            assert_eq!(tool_display::request(&call).1, fixture["normalized"]);
        }
    }
    assert!(
        matches!(session.handle(r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"list_terminals"}}"#),
        McpStep::Call { call: TerminalCall::ListTerminals, arguments, .. } if arguments == json!({}))
    );
}

#[test]
fn catalogue_and_mcp_reject_unknown_tools_bad_shapes_and_extra_fields() {
    let mut session = session();
    for (name, arguments) in [
        ("unknown", json!({})),
        ("list_terminals", json!({"future":true})),
        ("list_terminals", Value::Null),
        ("list_terminals", json!([])),
        ("read_terminal", json!({"terminal_id":"t1","future":true})),
        ("read_terminal", json!({"terminal_id":4})),
        (
            "send_input",
            json!({"terminal_id":"t1","text":"ls","press_enter":"yes"}),
        ),
        (
            "exec_command",
            json!({"terminal_id":"t1","program":"pwd","args":[1]}),
        ),
        ("open_terminal", json!({"server_id":null})),
        ("read_command", json!({"terminal_id":"t1"})),
    ] {
        assert!(
            get(name)
                .and_then(|spec| (spec.parse)(arguments.clone()))
                .is_none()
        );
        assert!(tool_display::parse(name, arguments.clone()).is_none());
        let McpStep::Rejected {
            id,
            tool,
            arguments: raw,
            error,
        } = invoke(&mut session, name, &arguments, json!("bad"))
        else {
            panic!("expected rejection");
        };
        assert_eq!(id, json!("bad"));
        assert_eq!(tool, name);
        assert_eq!(raw, arguments);
        assert_eq!(error, "Unknown tool or invalid arguments");
    }
}

#[test]
fn shape_parsing_remains_separate_from_execution_validation_and_identity_matching() {
    let arguments = json!({"terminal_id":"t1","lines":2001});
    let parsed = (get("read_terminal").unwrap().parse)(arguments.clone()).unwrap();
    assert!(parsed.validate().is_err());
    assert!(tool_display::parse("read_terminal", arguments.clone()).is_none());
    let row = acp::ToolCall::new("row", "mcp.nocterm-7.read_terminal").raw_input(arguments.clone());
    assert!(tool_display::requested_call(&row).is_some());
    assert!(tool_display::envelope(&row, "nocterm-7").is_none());
    let McpStep::Rejected {
        id,
        tool,
        arguments: raw,
        error,
    } = invoke(&mut session(), "read_terminal", &arguments, json!(43))
    else {
        panic!("expected validation rejection");
    };
    assert_eq!(id, json!(43));
    assert_eq!(tool, "read_terminal");
    assert_eq!(raw, arguments);
    assert_eq!(error, "Read limit must be 1–2000 lines");
}

#[test]
fn both_metadata_versions_use_the_same_shape_parser_without_losing_outcomes() {
    for fixture in fixtures() {
        for version in [1, 2] {
            let mut display = ToolDisplay::requested(
                fixture["name"].as_str().unwrap().into(),
                fixture["arguments"].clone(),
            );
            display.version = version;
            display.outcome = (version == 2).then(|| ToolOutcome::Ok(json!({"text":"ok"})));
            let mut row = acp::ToolCall::new("row", "unrecognized provider");
            display.attach(&mut row);
            let restored = ToolDisplay::from_call(&row).unwrap();
            assert_eq!(restored.version, version);
            assert!(correct_variant(
                &display.tool,
                &restored.display_request().unwrap()
            ));
            assert_eq!(
                serde_json::to_value(restored.outcome).unwrap(),
                serde_json::to_value(display.outcome).unwrap()
            );
        }
    }
}
