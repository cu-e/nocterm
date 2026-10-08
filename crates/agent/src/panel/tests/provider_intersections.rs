//! Real ACP and bridge routing intersections, without manually attaching display metadata.
use super::*;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::{ExecCommand, ReadCommand, TerminalCall, thread::Entry, tool_display};
use nocterm_session::{ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use nocterm_settings::ApprovalPolicy;
use serde_json::{Value, json};

#[derive(Default)]
struct Program {
    request: Mutex<Option<ExecRequest>>,
    sink: Mutex<Option<ExecSink>>,
}

impl HostExec for Program {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        *self.request.lock().unwrap() = Some(request);
        let (sink, output) = ExecOutput::channel();
        *self.sink.lock().unwrap() = Some(sink);
        async move { Ok(output) }.boxed()
    }
}

fn bridge(f: &Fixture, call: TerminalCall, cx: &mut TestAppContext) -> Value {
    let registration_id = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .registration()
            .as_ref()
            .unwrap()
            .id
    });
    let (respond, mut receive) = oneshot::channel();
    f.calls
        .try_send(BridgeCall {
            registration_id,
            call,
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    receive
        .try_recv()
        .unwrap()
        .expect("bridge answered")
        .unwrap()
}

fn update(
    f: &Fixture,
    session: &acp::SessionId,
    update: acp::SessionUpdate,
    cx: &mut TestAppContext,
) {
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            update,
        )))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn screenshot_identity_and_real_mcp_result_survive_late_input_and_history(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.bridge.next.store(17, Ordering::SeqCst);
    let program = Arc::new(Program::default());
    *f.access.executor.borrow_mut() = Some(program.clone());
    cx.update(|cx| {
        nocterm_ui::edit_settings(cx, |settings| {
            settings.ai.approval.terminal_write = ApprovalPolicy::Allow;
        })
        .detach()
    });
    cx.run_until_parked();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let terminal_id =
        cx.update(|cx| thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone()));
    let script = "printf '%s\\n' '*literal*' \"$HOME\"\r\n\tprintf '```'\n";
    let args = vec![
        "-lc".to_owned(),
        script.to_owned(),
        String::new(),
        "a b".to_owned(),
    ];
    let request = ExecCommand {
        terminal_id: terminal_id.clone(),
        program: "bash".into(),
        args: args.clone(),
        stdin: Some("input\r\n".into()),
        timeout_ms: Some(10000),
        yield_ms: Some(0),
    };
    let payload = bridge(&f, TerminalCall::ExecCommand(request.clone()), cx);
    let actual = program.request.lock().unwrap();
    let actual = actual
        .as_ref()
        .expect("exact attached executor received request");
    assert_eq!(actual.program, "bash");
    assert_eq!(actual.args, args);
    assert_eq!(actual.stdin.as_deref(), Some(&b"input\r\n"[..]));
    update(
        &f,
        &session,
        acp::SessionUpdate::ToolCall(acp::ToolCall::new(
            "provider-call",
            "mcp__nocterm-17__exec_command",
        )),
        cx,
    );
    cx.update(|cx| {
        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
            unreachable!()
        };
        assert!(
            tool_display::ToolDisplay::from_call(call).is_none(),
            "name alone cannot establish the executed host"
        );
    });
    let stdout = "line\r\n\t```literal```\n界\n";
    let sink = program.sink.lock().unwrap().take().unwrap();
    assert!(futures::executor::block_on(
        sink.send(stdout.as_bytes().to_vec())
    ));
    sink.finish(ExecExit {
        status: Some(7),
        stderr: "warning\n".into(),
    });
    drop(sink);
    cx.run_until_parked();
    let result = bridge(
        &f,
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id,
            command_id: payload["command_id"].as_str().unwrap().into(),
            yield_ms: Some(0),
        }),
        cx,
    );
    assert_eq!(result["state"], "exited");
    let wire = nocterm_ai::mcp::tool_result(json!(42), Ok(result.clone()));
    update(
        &f,
        &session,
        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
            "provider-call",
            acp::ToolCallUpdateFields::new()
                .raw_input(serde_json::to_value(&request).unwrap())
                .raw_output(wire["result"].clone())
                .content(
                    serde_json::from_value::<Vec<acp::ToolCallContent>>(
                        json!([{"type":"content","content":{
                            "type":"text","text":result.to_string()
                        }}]),
                    )
                    .unwrap(),
                )
                .status(acp::ToolCallStatus::Completed),
        )),
        cx,
    );
    cx.update(|cx| {
        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
            unreachable!()
        };
        assert_eq!(
            tool_display::header(call),
            "This computer · Execute program · Completed"
        );
        let restored: acp::ToolCall =
            serde_json::from_value(serde_json::to_value(call).unwrap()).unwrap();
        assert_eq!(tool_display::header(&restored), tool_display::header(call));
        let input = tool_input::source(&restored).unwrap();
        assert_eq!(input.text, script);
        assert_eq!(input.language, Some("bash"));
        let output = tool_output::source(&restored, false).unwrap();
        assert_eq!(
            output.sections.len(),
            2,
            "same MCP result is not rendered twice"
        );
        assert_eq!(output.sections[0].text, stdout);
        assert_eq!(output.sections[1].text, "warning\n");
        assert!(
            output
                .parameters
                .iter()
                .any(|text| text == "Process: Exited")
        );
        assert!(
            output
                .parameters
                .iter()
                .any(|text| text == "Exit status: 7")
        );
    });
    cx.update(|cx| thread.update(cx, |thread, _| thread.attachments.clear()));
    update(&f, &session, acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        "provider-call", acp::ToolCallUpdateFields::new().title("Different destination")
            .raw_input(json!({"terminal_id":"different","program":"bash","args":["-lc","echo forged"]})),
    )), cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("tool-call", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("tool-call", 0usize)).label(),
            Some("This computer · Execute program · Completed")
        );
        window.click(("copy-tool-input", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), script);
        window.click(("copy-tool-output", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), stdout);
    })
    .unwrap();
}

#[gpui_kit::test]
fn terminal_ask_switches_off_do_not_implicitly_approve_provider_requests(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        nocterm_ui::edit_settings(cx, |settings| {
            settings.ai.approval.terminal_read = ApprovalPolicy::Allow;
            settings.ai.approval.terminal_write = ApprovalPolicy::Allow;
        })
        .detach()
    });
    cx.run_until_parked();
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let (respond, mut receive) = nocterm_ai::PermissionResponder::channel();
    f.events
        .try_send(AgentEvent::Permission {
            request: serde_json::from_value(json!({"sessionId":session,"toolCall":{
            "toolCallId":"permission","title":"Provider action"
        },"options":[{"optionId":"no","name":"Yes","kind":"reject_once"},
            {"optionId":"yes","name":"No","kind":"allow_once"}]}))
            .unwrap(),
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    assert!(receive.try_recv().unwrap().is_none());
    cx.update(|cx| assert_eq!(thread.read(cx).permissions.len(), 1));
    cx.update(|cx| {
        nocterm_ui::edit_settings(cx, |settings| {
            settings.ai.approval.agent_permissions = ApprovalPolicy::Allow;
        })
        .detach()
    });
    cx.run_until_parked();
    assert!(
        matches!(receive.try_recv().unwrap(), Some(acp::RequestPermissionOutcome::Selected(choice))
        if choice.option_id == acp::PermissionOptionId::from("yes"))
    );
    cx.update(|cx| assert!(thread.read(cx).permissions.is_empty()));
}
