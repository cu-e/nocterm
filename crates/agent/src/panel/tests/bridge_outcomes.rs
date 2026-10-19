//! Bridge-owned outcomes are independent of provider result envelopes.
use super::provider_fixtures::{known_title, provider_tool};
use super::*;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::model::{
    controls::Presence,
    json::OwnedJson,
    literal::{LiteralIdentity, TextRange},
    retention::RetentionAuthority,
    tools::{Claim, ToolRecord, ToolStatus},
};
use nocterm_ai::{TerminalCall, tool_display};
use serde_json::{Value, json};
fn provider_row(
    authority: &RetentionAuthority,
    id: &str,
    provider: &str,
    tool: &str,
    arguments: Value,
    result: Option<Value>,
) -> ToolRecord {
    let mut row =
        super::provider_fixtures::provider_tool(id, title(provider, tool), ToolStatus::Pending);
    let (owner, start) = match provider {
        "codex" => (12..14, 15),
        "claude" => (13..15, 17),
        _ => (12..14, 15),
    };
    let end = row.title.len();
    super::provider_fixtures::known_title(&mut row, owner, start..end);
    row.arguments = Some(OwnedJson::from_untracked(authority, arguments).unwrap());
    row.result = result.map(|value| OwnedJson::from_untracked(authority, value).unwrap());
    row
}

fn title(provider: &str, tool: &str) -> String {
    match provider {
        "codex" => format!("mcp.nocterm-17.{tool}"),
        "claude" => format!("mcp__nocterm-17__{tool}"),
        _ => format!("mcp_nocterm_17_{tool}"),
    }
}

fn send(
    f: &Fixture,
    request: TerminalCall,
    arguments: Value,
    cx: &mut TestAppContext,
) -> oneshot::Receiver<Result<Value, nocterm_ai::ToolFailure>> {
    let (respond, receive) = oneshot::channel();
    f.calls
        .try_send(BridgeCall {
            registration_id: 17,
            call: request,
            arguments: Some(arguments),
            display_token: None,
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    receive
}

fn assert_error(thread: &Entity<crate::thread::AgentThread>, index: usize, cx: &App) {
    let call = thread.read(cx).tool_slot(index);
    assert!(tool_display::managed_header(call).contains("Execute program"));
    assert_eq!(
        tool_input::source_managed(call).unwrap().label,
        "Requested command"
    );
    let output = tool_output::source_managed(call, false).unwrap();
    assert_eq!(output.sections.len(), 1);
    assert_eq!(output.sections[0].label, "Error");
    assert!(!output.sections[0].text.contains("provider garbage"));
}

#[gpui_kit::test]
fn every_provider_uses_bridge_success_validation_rejection_and_denial(cx: &mut TestAppContext) {
    for provider in ["codex", "claude", "hermes"] {
        check_provider(provider, cx);
    }
}

fn check_provider(provider: &str, cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.bridge.next.store(17, Ordering::SeqCst);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let terminal = cx.update(|cx| thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone()));
    for (id, tool, args) in [
        ("success", "list_terminals", json!({})),
        (
            "mcp-rejection",
            "exec_command",
            json!({"terminal_id":terminal,"program":"sh","args":["-c","echo requested"],"timeout_ms":1_800_000}),
        ),
        (
            "handle-rejection",
            "exec_command",
            json!({"terminal_id":terminal,"program":"sh","args":["-c","echo handle requested"],"timeout_ms":1_800_000}),
        ),
        (
            "denial",
            "exec_command",
            json!({"terminal_id":terminal,"program":"sh","args":["-c","echo requested"]}),
        ),
    ] {
        let row = provider_row(
            &f.installation.retention,
            id,
            provider,
            tool,
            args.clone(),
            Some(json!({"new_provider_wrapper":"provider garbage"})),
        );
        deliver(&f, &session, SessionUpdate::ToolStarted(row), cx);
        if id == "mcp-rejection" {
            let mut mcp = nocterm_ai::mcp::McpSession::default();
            mcp.handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
            let nocterm_ai::mcp::McpStep::Rejected { tool, arguments, error, .. } =
                mcp.handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":args}}).to_string())
            else { panic!("rejected"); };
            f.rejections
                .try_send(nocterm_ai::BridgeRejection {
                    registration_id: 17,
                    tool,
                    arguments,
                    error,
                })
                .unwrap();
            cx.run_until_parked();
        } else {
            let request = if tool == "list_terminals" {
                TerminalCall::ListTerminals
            } else {
                TerminalCall::ExecCommand(serde_json::from_value(args.clone()).unwrap())
            };
            let mut receive = send(&f, request, args, cx);
            if id == "denial" {
                assert!(receive.try_recv().unwrap().is_none());
                cx.update(|cx| {
                    thread.update(cx, |thread, cx| {
                        thread.approve_tool(thread.tool_cards()[0].token(), false, false, cx)
                    })
                });
            }
            assert!(receive.try_recv().unwrap().is_some());
        }
    }
    cx.update(|cx| {
        let call = thread.read(cx).tool_slot(0);
        let output = tool_output::source_managed(call, false).unwrap();
        assert_eq!(output.sections[0].label, "Context");
        assert!(!output.sections[0].text.contains("provider garbage"));
        for index in 1..4 {
            assert_error(&thread, index, cx);
        }
    });
}

#[gpui_kit::test]
fn stopping_an_active_request_persists_an_error_without_overwriting_completed_results(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.bridge.next.store(17, Ordering::SeqCst);
    *f.access.executor().borrow_mut() =
        Some(Arc::new(super::provider_fixtures::Program::default()));
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.approval.terminal_write = nocterm_ai::ApprovalPolicy::Allow;
            })
            .detach()
    });
    cx.run_until_parked();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let terminal = cx.update(|cx| thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone()));
    let arguments =
        json!({"terminal_id":terminal,"program":"sh","args":["-c","echo active"],"yield_ms":1000});
    deliver(
        &f,
        &session,
        SessionUpdate::ToolStarted(provider_row(
            &f.installation.retention,
            "active",
            "codex",
            "exec_command",
            arguments.clone(),
            None,
        )),
        cx,
    );
    let mut pending = send(
        &f,
        TerminalCall::ExecCommand(serde_json::from_value(arguments.clone()).unwrap()),
        arguments,
        cx,
    );
    assert!(pending.try_recv().unwrap().is_none());
    deliver(
        &f,
        &session,
        SessionUpdate::ToolStarted(provider_row(
            &f.installation.retention,
            "complete",
            "codex",
            "list_terminals",
            json!({}),
            None,
        )),
        cx,
    );
    assert!(
        send(&f, TerminalCall::ListTerminals, json!({}), cx)
            .try_recv()
            .unwrap()
            .is_some()
    );
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.stop(cx);
            let active = thread.tool_slot(0);
            assert!(matches!(
                tool_display::ToolDisplay::from_managed(active)
                    .unwrap()
                    .outcome,
                Presence::Value(tool_display::ToolOutcome::Failed(_))
            ));
            let complete = thread.tool_slot(1);
            assert!(matches!(
                tool_display::ToolDisplay::from_managed(complete)
                    .unwrap()
                    .outcome,
                Presence::Value(tool_display::ToolOutcome::Succeeded(_))
            ));
        })
    });
}

#[gpui_kit::test]
fn queued_rejections_cannot_attach_after_the_chat_stops(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.bridge.next.store(17, Ordering::SeqCst);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    deliver(
        &f,
        &session,
        SessionUpdate::ToolStarted(provider_row(
            &f.installation.retention,
            "late",
            "codex",
            "typo",
            json!({}),
            None,
        )),
        cx,
    );
    cx.update(|cx| thread.update(cx, |thread, cx| thread.stop(cx)));
    f.rejections
        .try_send(nocterm_ai::BridgeRejection {
            registration_id: 17,
            tool: "typo".into(),
            arguments: json!({}),
            error: "late rejection".into(),
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let call = thread.read(cx).tool_slot(0);
        assert!(tool_display::ToolDisplay::from_managed(call).is_none());
    });
}

#[test]
fn bridge_error_metadata_stays_sanitized_when_display_redaction_is_disabled() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = super::provider_fixtures::provider_tool(
        "error",
        title("codex", "list_terminals"),
        ToolStatus::Pending,
    );
    let mut display =
        tool_display::ToolDisplay::requested(&authority, "list_terminals".into(), json!({}))
            .unwrap();
    display
        .finish(Err("password=private-value".into()))
        .unwrap();
    display.attach(&mut call);
    let output = tool_output::source(&call, false).unwrap();
    assert_eq!(output.sections[0].label, "Error");
    assert!(!output.sections[0].text.contains("private-value"));
    assert!(
        !serde_json::to_string(&call.client_display)
            .unwrap()
            .contains("private-value")
    );
}
#[test]
fn a_call_the_bridge_rejected_shows_the_command_and_the_error() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let mut call = provider_tool("call-1", "mcp.nocterm-11.exec_command", ToolStatus::Failed);
    let title_len = call.title.len();
    known_title(&mut call, 12..14, 15..title_len);
    call.arguments = Some(OwnedJson::from_untracked(&authority, json!({"server":"nocterm-11", "tool":"exec_command", "arguments":{
        "terminal_id":"t1", "program":"/bin/sh", "args":["-c","cd ~/dev/NOUMMA && cargo build --release --locked -j 4"], "timeout_ms":1_800_000, "yield_ms":1000,
    }})).unwrap());
    let input = call.arguments.as_ref().unwrap();
    let server = input["server"].as_str().unwrap();
    let tool = input["tool"].as_str().unwrap();
    call.literal.input = LiteralIdentity::Registered {
        owner: TextRange::supplied(server, &server[8..]).unwrap(),
        tool: TextRange::supplied(tool, tool).unwrap(),
    };
    call.claims.input = Claim::Known {
        registration: "11".into(),
        tool: "exec_command".into(),
    };
    call.result = Some(
        OwnedJson::from_untracked(
            &authority,
            json!({"result":null, "error":{
                "message":"tool call error: tool call failed for `nocterm-11/exec_command`"
            }}),
        )
        .unwrap(),
    );
    assert_eq!(
        tool_display::header(&call),
        "Nocterm · Execute program · Failed"
    );
    let input = tool_input::source(&call).unwrap();
    assert_eq!(input.label, "Requested command");
    assert_eq!(
        input.text,
        "cd ~/dev/NOUMMA && cargo build --release --locked -j 4"
    );
    let output = tool_output::source(&call, false).unwrap();
    assert_eq!(output.sections.len(), 1);
    assert_eq!(output.sections[0].label, "Error");
    assert!(output.sections[0].text.starts_with("tool call error"));
}
