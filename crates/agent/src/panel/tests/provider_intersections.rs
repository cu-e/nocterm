//! Real ACP and bridge routing intersections, without manually attaching display metadata.
use super::*;
use crate::panel::entries::{tool_input, tool_output};
use nocterm_ai::ApprovalPolicy;
use nocterm_ai::model::{
    ids::ToolId,
    json::OwnedJson,
    literal::{LiteralIdentity, TextRange},
    tools::{Claim, ToolChanges, ToolOutput, ToolPatch, ToolRecord, ToolStatus},
};
use nocterm_ai::{ExecCommand, ReadCommand, TerminalCall, thread::Entry, tool_display};
use nocterm_session::{ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use serde_json::{Value, json};

/// Actual neutral provider facts before any optional payload or bridge evidence is supplied.
pub(super) fn provider_tool(
    id: impl Into<String>,
    title: impl Into<String>,
    status: nocterm_ai::model::tools::ToolStatus,
) -> nocterm_ai::model::tools::ToolRecord {
    use nocterm_ai::model::{
        ids::ToolId,
        tools::{ToolKind, ToolRecord},
    };
    ToolRecord {
        id: ToolId::new(id),
        title: title.into(),
        name: None,
        kind: ToolKind::Other,
        status,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    }
}

/// These fixtures supply known neutral facts and genuine ranges in their retained title.
/// No wire parser or execution authority is created by this test constructor.
pub(super) fn known_title(
    call: &mut ToolRecord,
    owner: std::ops::Range<usize>,
    tool: std::ops::Range<usize>,
) {
    let registration = call.title[owner.clone()].to_owned();
    let name = call.title[tool.clone()].to_owned();
    call.literal.title = LiteralIdentity::Registered {
        owner: TextRange::supplied(&call.title, &call.title[owner]).unwrap(),
        tool: TextRange::supplied(&call.title, &call.title[tool]).unwrap(),
    };
    call.claims.title = Claim::Known {
        registration,
        tool: name,
    };
}

#[derive(Default)]
pub(super) struct Program {
    pub(super) request: Mutex<Option<ExecRequest>>,
    pub(super) sink: Mutex<Option<ExecSink>>,
}

impl HostExec for Program {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        *self.request.lock().unwrap() = Some(request);
        let (sink, output) = ExecOutput::channel();
        *self.sink.lock().unwrap() = Some(sink);
        async move { Ok(output) }.boxed()
    }
}

pub(super) fn bridge(f: &Fixture, call: TerminalCall, cx: &mut TestAppContext) -> Value {
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
            arguments: None,
            display_token: None,
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

pub(super) fn update(
    f: &Fixture,
    session: &SessionId,
    update: SessionUpdate,
    cx: &mut TestAppContext,
) {
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session: session.clone(),
            update,
        }))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn screenshot_identity_and_real_mcp_result_survive_late_input_and_history(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.bridge.next.store(17, Ordering::SeqCst);
    let program = Arc::new(Program::default());
    *f.access.executor().borrow_mut() = Some(program.clone());
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.approval.terminal_write = ApprovalPolicy::Allow;
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
    let authority = cx.update(|cx| f.installation.installed.runtime().retention(cx).clone());
    let mut call = provider_tool(
        "provider-call",
        "mcp__nocterm-17__exec_command",
        ToolStatus::Pending,
    );
    let title_len = call.title.len();
    known_title(&mut call, 13..15, 17..title_len);
    update(&f, &session, SessionUpdate::ToolStarted(call), cx);
    cx.update(|cx| {
        let call = thread.read(cx).tool_slot(0);
        assert!(
            tool_display::ToolDisplay::from_managed(call).is_none(),
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
    let patch = ToolPatch {
        id: ToolId::new("provider-call"),
        changes: ToolChanges {
            arguments: Some(
                OwnedJson::from_untracked(&authority, serde_json::to_value(&request).unwrap())
                    .unwrap(),
            ),
            result: Some(OwnedJson::from_untracked(&authority, wire["result"].clone()).unwrap()),
            output: Some(vec![ToolOutput::Content {
                content: Box::new(Content::Text(Text {
                    text: result.to_string(),
                    annotations: None,
                    extensions: None,
                })),
                extensions: None,
            }]),
            status: Some(ToolStatus::Succeeded),
            ..Default::default()
        },
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
    };
    update(&f, &session, SessionUpdate::ToolChanged(patch), cx);
    cx.update(|cx| {
        let call = thread.read(cx).tool_slot(0);
        assert_eq!(
            tool_display::managed_header(call),
            "This computer · Execute program · Completed"
        );
        let snapshot = thread.read(cx).snapshot(cx).unwrap();
        f.repository.seed_chat(&snapshot).unwrap();
        let restored_chat = f.repository.transcript(snapshot.id()).unwrap();
        let Entry::Tool(restored) = &restored_chat.transcript().entries[0] else {
            panic!("restored tool row");
        };
        assert_eq!(
            tool_display::header(restored),
            tool_display::managed_header(call)
        );
        let input = tool_input::source(restored).unwrap();
        assert_eq!(input.text, script);
        assert_eq!(input.language, Some("bash"));
        let output = tool_output::source(restored, false).unwrap();
        assert_eq!(
            output.sections.len(),
            2,
            "same MCP result is not rendered twice"
        );
        assert_eq!(output.sections[0].text, payload["stdout"].as_str().unwrap());
        assert_eq!(output.sections[1].text, payload["stderr"].as_str().unwrap());
        assert!(output.parameters.iter().any(|text| text
            == &format!(
                "Process: {}",
                if payload["state"] == "running" {
                    "Running"
                } else {
                    "Starting"
                }
            )));
        assert!(
            output
                .parameters
                .iter()
                .any(|text| text == "Exit status: unknown")
        );
    });
    cx.update(|cx| thread.update(cx, |thread, cx| thread.clear_composer_context(cx)));
    update(&f, &session, SessionUpdate::ToolChanged(ToolPatch {
        id: ToolId::new("provider-call"),
        changes: ToolChanges {
            title: Some("Different destination".into()),
            arguments: Some(OwnedJson::from_untracked(&authority, json!({"terminal_id":"different","program":"bash","args":["-lc","echo forged"]})).unwrap()),
            ..Default::default()
        },
        extensions: None, claims: Default::default(), literal: Default::default(),
    }), cx);
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
    })
    .unwrap();
}

#[gpui_kit::test]
fn terminal_ask_switches_off_do_not_implicitly_approve_provider_requests(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.approval.terminal_read = ApprovalPolicy::Allow;
                settings.approval.terminal_write = ApprovalPolicy::Allow;
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
            request: super::provider_permissions::permission(
                session,
                "permission",
                "Provider action",
                vec![
                    super::provider_permissions::choice(
                        "no",
                        "Yes",
                        nocterm_ai::model::auth::PermissionEffect::DenyOnce,
                    ),
                    super::provider_permissions::choice(
                        "yes",
                        "No",
                        nocterm_ai::model::auth::PermissionEffect::AllowOnce,
                    ),
                ],
            ),
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    assert!(receive.try_recv().unwrap().is_none());
    cx.update(|cx| assert_eq!(thread.read(cx).permission_cards().len(), 1));
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.approval.agent_permissions = ApprovalPolicy::Allow;
            })
            .detach()
    });
    cx.run_until_parked();
    assert!(
        matches!(receive.try_recv().unwrap(), Some(nocterm_ai::model::auth::PermissionOutcome::Selected(choice))
        if choice == nocterm_ai::model::ids::PermissionOptionId::new("yes"))
    );
    cx.update(|cx| assert!(thread.read(cx).permission_cards().is_empty()));
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
