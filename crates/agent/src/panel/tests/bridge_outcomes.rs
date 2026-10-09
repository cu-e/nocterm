//! Bridge-owned outcomes are independent of provider result envelopes.
use super::provider_intersections::update;
use super::*;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::{TerminalCall, thread::Entry, tool_display};
use serde_json::{Value, json};

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
) -> oneshot::Receiver<Result<Value, String>> {
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
    let Entry::Tool(call) = &thread.read(cx).state.entries[index] else {
        panic!("tool row");
    };
    assert!(tool_display::header(call).contains("Execute program"));
    assert_eq!(tool_input::source(call).unwrap().label, "Requested command");
    let output = tool_output::source(call, false).unwrap();
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
        let row = acp::ToolCall::new(id, title(provider, tool))
            .raw_input(args.clone())
            .raw_output(json!({"new_provider_wrapper":"provider garbage"}));
        update(&f, &session, acp::SessionUpdate::ToolCall(row), cx);
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
                    thread.update(cx, |thread, cx| thread.approve_tool(0, false, false, cx))
                });
            }
            assert!(receive.try_recv().unwrap().is_some());
        }
    }
    cx.update(|cx| {
        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
            panic!("tool row");
        };
        let output = tool_output::source(call, false).unwrap();
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
    *f.access.executor.borrow_mut() =
        Some(Arc::new(super::provider_intersections::Program::default()));
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
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
    update(
        &f,
        &session,
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("active", title("codex", "exec_command"))
                .raw_input(arguments.clone()),
        ),
        cx,
    );
    let mut pending = send(
        &f,
        TerminalCall::ExecCommand(serde_json::from_value(arguments.clone()).unwrap()),
        arguments,
        cx,
    );
    assert!(pending.try_recv().unwrap().is_none());
    update(
        &f,
        &session,
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("complete", title("codex", "list_terminals")).raw_input(json!({})),
        ),
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
            let Entry::Tool(active) = &thread.state.entries[0] else {
                panic!("tool row");
            };
            assert!(matches!(
                tool_display::ToolDisplay::from_call(active)
                    .unwrap()
                    .outcome,
                Some(tool_display::ToolOutcome::Err(_))
            ));
            let Entry::Tool(complete) = &thread.state.entries[1] else {
                panic!("tool row");
            };
            assert!(matches!(
                tool_display::ToolDisplay::from_call(complete)
                    .unwrap()
                    .outcome,
                Some(tool_display::ToolOutcome::Ok(_))
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
    update(
        &f,
        &session,
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("late", title("codex", "typo")).raw_input(json!({})),
        ),
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
        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
            panic!("tool row");
        };
        assert!(tool_display::ToolDisplay::from_call(call).is_none());
    });
}

#[test]
fn bridge_error_metadata_stays_sanitized_when_display_redaction_is_disabled() {
    let mut call = acp::ToolCall::new("error", title("codex", "list_terminals"));
    let mut display = tool_display::ToolDisplay::requested("list_terminals".into(), json!({}));
    display.finish(Err("password=private-value".into()));
    display.attach(&mut call);
    let output = tool_output::source(&call, false).unwrap();
    assert_eq!(output.sections[0].label, "Error");
    assert!(!output.sections[0].text.contains("private-value"));
    assert!(
        !serde_json::to_string(&call.meta)
            .unwrap()
            .contains("private-value")
    );
}
