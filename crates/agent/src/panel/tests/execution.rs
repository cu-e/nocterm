//! Exercise command ownership through the same bridge and chat lifecycle as users.
use super::*;
use crate::thread::AgentThread;
use nocterm_ai::{CancelCommand, ExecCommand, ReadCommand, TerminalCall};
use nocterm_session::{ExecExit, ExecFuture, ExecOutput, ExecRequest, ExecSink, HostExec};
use serde_json::Value;
use std::time::Duration;

#[derive(Default)]
struct Programs {
    requests: Mutex<Vec<ExecRequest>>,
    sinks: Mutex<Vec<Option<Arc<ExecSink>>>>,
}

impl HostExec for Programs {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        self.requests.lock().unwrap().push(request);
        let (sink, output) = ExecOutput::channel();
        self.sinks.lock().unwrap().push(Some(Arc::new(sink)));
        async move { Ok(output) }.boxed()
    }
}

impl Programs {
    fn closed(&self, index: usize) -> bool {
        self.sinks.lock().unwrap()[index]
            .as_ref()
            .unwrap()
            .is_closed()
    }

    fn finish(&self, index: usize, stdout: &[u8], status: u32, stderr: &str) {
        let sink = self.sinks.lock().unwrap()[index].take().unwrap();
        assert!(futures::executor::block_on(sink.send(stdout.to_vec())));
        sink.finish(ExecExit {
            status: Some(status),
            stderr: stderr.into(),
        });
    }
}

type Reply = oneshot::Receiver<Result<Value, nocterm_ai::ToolFailure>>;

fn current(f: &Fixture, cx: &mut TestAppContext) -> Entity<AgentThread> {
    cx.update(|cx| f.panel.read(cx).current().unwrap())
}

fn terminal_id(thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone()))
}

fn submit(
    f: &Fixture,
    thread: &Entity<AgentThread>,
    call: TerminalCall,
    approve: bool,
    cx: &mut TestAppContext,
) -> Reply {
    let registration_id = cx.update(|cx| thread.read(cx).registration().as_ref().unwrap().id);
    let (respond, response) = oneshot::channel();
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
    if approve {
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                if !thread.tool_cards().is_empty() {
                    assert_eq!(thread.tool_cards().len(), 1);
                    thread.approve_tool(thread.tool_cards()[0].token(), true, false, cx);
                }
            })
        });
        cx.run_until_parked();
    }
    response
}

fn result(reply: Reply) -> Result<Value, nocterm_ai::ToolFailure> {
    reply
        .now_or_never()
        .expect("tool responded")
        .expect("bridge response retained")
}

fn request(terminal_id: &str) -> ExecCommand {
    ExecCommand {
        terminal_id: terminal_id.into(),
        program: "demo-program".into(),
        args: vec![
            "argument with spaces".into(),
            "'quote'".into(),
            "$(literal)".into(),
            "".into(),
        ],
        stdin: Some("first\nsecond\n".into()),
        timeout_ms: Some(30_000),
        yield_ms: Some(0),
    }
}

fn setup(cx: &mut TestAppContext) -> (Fixture, Entity<AgentThread>, Arc<Programs>, String) {
    let f = fixture(cx);
    let programs = Arc::new(Programs::default());
    *f.access.executor().borrow_mut() = Some(programs.clone());
    new_chat(&f, cx);
    let thread = current(&f, cx);
    let terminal = terminal_id(&thread, cx);
    (f, thread, programs, terminal)
}

fn start(
    f: &Fixture,
    thread: &Entity<AgentThread>,
    terminal: &str,
    cx: &mut TestAppContext,
) -> String {
    let payload = result(submit(
        f,
        thread,
        TerminalCall::ExecCommand(request(terminal)),
        true,
        cx,
    ))
    .unwrap();
    assert!(
        matches!(payload["state"].as_str(), Some("starting" | "running")),
        "{payload}"
    );
    payload["command_id"].as_str().unwrap().into()
}

fn read(
    f: &Fixture,
    thread: &Entity<AgentThread>,
    terminal: &str,
    id: &str,
    cx: &mut TestAppContext,
) -> Result<Value, nocterm_ai::ToolFailure> {
    result(submit(
        f,
        thread,
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id: terminal.into(),
            command_id: id.into(),
            yield_ms: Some(0),
        }),
        true,
        cx,
    ))
}

#[gpui_kit::test]
fn structured_requests_preserve_arguments_and_report_real_exit_with_redaction(
    cx: &mut TestAppContext,
) {
    let (f, thread, programs, terminal) = setup(cx);
    let id = start(&f, &thread, &terminal, cx);
    let requests = programs.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program, request(&terminal).program);
    assert_eq!(requests[0].args, request(&terminal).args);
    assert_eq!(requests[0].stdin.as_deref(), Some(&b"first\nsecond\n"[..]));
    drop(requests);
    programs.finish(
        0,
        b"output\nDB_PASSWORD=hunter2\n",
        7,
        "api_key=secret-value",
    );
    cx.run_until_parked();
    let payload = read(&f, &thread, &terminal, &id, cx).unwrap();
    assert_eq!(payload["state"], "exited");
    assert_eq!(payload["exit_status"], 7);
    assert!(payload["stdout"].as_str().unwrap().contains("output"));
    assert!(!payload.to_string().contains("hunter2"));
    assert!(!payload.to_string().contains("secret-value"));
    assert_eq!(payload["truncated"], false);
    assert!(
        f.access.sent().borrow().is_empty(),
        "structured execution never types into the terminal"
    );
}

#[gpui_kit::test]
fn a_silent_program_and_an_abandoned_read_remain_running_until_cancelled(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let id = start(&f, &thread, &terminal, cx);
    let abandoned = submit(
        &f,
        &thread,
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id: terminal.clone(),
            command_id: id.clone(),
            yield_ms: Some(10_000),
        }),
        false,
        cx,
    );
    drop(abandoned);
    cx.run_until_parked();
    assert!(!programs.closed(0));
    let payload = read(&f, &thread, &terminal, &id, cx).unwrap();
    assert_eq!(payload["state"], "running");
    assert!(payload["exit_status"].is_null());
    let cancelled = result(submit(
        &f,
        &thread,
        TerminalCall::CancelCommand(CancelCommand {
            terminal_id: terminal.clone(),
            command_id: id.clone(),
        }),
        true,
        cx,
    ))
    .unwrap();
    assert_eq!(cancelled["state"], "cancelled");
    cx.run_until_parked();
    assert!(programs.closed(0), "cancelling drops the transport output");
    assert_eq!(
        read(&f, &thread, &terminal, &id, cx).unwrap()["state"],
        "cancelled"
    );
}

#[gpui_kit::test]
fn approval_rechecks_authentication_and_unsupported_execution_never_falls_back(
    cx: &mut TestAppContext,
) {
    let (f, thread, programs, terminal) = setup(cx);
    let pending = submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(request(&terminal)),
        false,
        cx,
    );
    assert_eq!(cx.update(|cx| thread.read(cx).tool_cards().len()), 1);
    f.access.sign_in().asks().set(true);
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.approve_tool(thread.tool_cards()[0].token(), true, false, cx)
        })
    });
    cx.run_until_parked();
    assert!(
        result(pending)
            .unwrap_err()
            .message()
            .contains("authentication")
    );
    assert!(programs.requests.lock().unwrap().is_empty());
    f.access.sign_in().asks().set(false);
    f.access.executor().borrow_mut().take();
    let unsupported = result(submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(request(&terminal)),
        true,
        cx,
    ));
    assert_eq!(
        unsupported.unwrap_err().message(),
        "This terminal does not support structured execution."
    );
    assert!(programs.requests.lock().unwrap().is_empty());
    assert!(f.access.sent().borrow().is_empty());
}

#[gpui_kit::test]
fn command_handles_are_owned_by_the_chat_that_started_them(cx: &mut TestAppContext) {
    let (f, first, programs, terminal) = setup(cx);
    let id = start(&f, &first, &terminal, cx);
    cx.update(|cx| first.update(cx, |thread, cx| thread.rename(Some("first".into()), cx)));
    new_chat(&f, cx);
    let second = current(&f, cx);
    let second_terminal = terminal_id(&second, cx);
    assert_eq!(
        read(&f, &second, &second_terminal, &id, cx)
            .unwrap_err()
            .message(),
        "Unknown command for this chat and terminal."
    );
    assert!(!programs.closed(0));
    assert_eq!(
        read(&f, &first, &terminal, &id, cx).unwrap()["state"],
        "running"
    );
}

#[gpui_kit::test]
fn active_command_limit_and_cancelled_slot_apply_through_bridge_calls(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(start(&f, &thread, &terminal, cx));
    }
    let fifth = result(submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(request(&terminal)),
        true,
        cx,
    ));
    assert_eq!(
        fifth.unwrap_err().message(),
        "This chat already has four active commands. Read or cancel one first."
    );
    assert_eq!(programs.requests.lock().unwrap().len(), 4);
    result(submit(
        &f,
        &thread,
        TerminalCall::CancelCommand(CancelCommand {
            terminal_id: terminal.clone(),
            command_id: ids[0].clone(),
        }),
        true,
        cx,
    ))
    .unwrap();
    let next = start(&f, &thread, &terminal, cx);
    assert!(!ids.contains(&next));
    assert_eq!(programs.requests.lock().unwrap().len(), 5);
    assert!(programs.closed(0));
}

#[gpui_kit::test]
fn stop_cancels_owned_programs_even_when_no_prompt_is_generating(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let id = start(&f, &thread, &terminal, cx);
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            assert!(!thread.session_generating());
            thread.stop(cx);
            assert!(thread.session_stopped());
            assert!(
                thread.session_accepts_updates(),
                "idle Stop preserves access to retained command results"
            );
        })
    });
    cx.run_until_parked();
    assert!(programs.closed(0));
    assert_eq!(
        read(&f, &thread, &terminal, &id, cx).unwrap()["state"],
        "cancelled"
    );
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.approval.agent_permissions = nocterm_ai::ApprovalPolicy::Allow;
            })
            .detach();
    });
    cx.run_until_parked();
    let (respond, mut permission) = nocterm_ai::PermissionResponder::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, _| {
            let request = nocterm_ai::model::auth::ProviderPermission {
                session: thread.session().clone().unwrap(),
                tool: nocterm_ai::model::tools::ToolPatch {
                    id: nocterm_ai::model::ids::ToolId::new("late-provider-tool"),
                    changes: Default::default(),
                    extensions: None,
                    claims: Default::default(),
                    literal: Default::default(),
                },
                choices: vec![super::provider_permissions::choice(
                    "once",
                    "Allow once",
                    nocterm_ai::model::auth::PermissionEffect::AllowOnce,
                )],
                extensions: None,
            };
            f.events
                .try_send(AgentEvent::Permission { request, respond })
                .unwrap();
        })
    });
    cx.run_until_parked();
    assert_eq!(
        permission.try_recv().unwrap(),
        Some(nocterm_ai::model::auth::PermissionOutcome::Cancelled)
    );
    assert_eq!(
        read(&f, &thread, &terminal, &id, cx).unwrap()["state"],
        "cancelled"
    );
}

#[gpui_kit::test]
fn releasing_acp_while_generating_preserves_document_owned_execution(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let _id = start(&f, &thread, &terminal, cx);
    thread.update(cx, |thread, cx| thread.send("continue".into(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(thread.read(cx).session_generating()));
    thread.update(cx, |thread, cx| thread.release_resources(cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!thread.read(cx).is_leased());
        assert!(!thread.read(cx).session_generating());
        assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1);
    });
    assert!(
        !programs.closed(0),
        "releasing ACP cancelled a document-owned program"
    );
    assert!(f.access.sent().borrow().is_empty());
    programs.finish(0, b"finished after ACP close", 0, "");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn removing_the_active_attachment_cancels_owned_programs(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let id = start(&f, &thread, &terminal, cx);
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Terminal(f.terminal), cx)
        })
    });
    cx.run_until_parked();
    assert!(programs.closed(0));
    assert!(read(&f, &thread, &terminal, &id, cx).is_err());
}

#[gpui_kit::test]
fn replacing_a_session_executor_revokes_old_commands_and_allows_new_ones(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let old = start(&f, &thread, &terminal, cx);
    let replacement = Arc::new(Programs::default());
    *f.access.executor().borrow_mut() = Some(replacement.clone());
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert!(programs.closed(0));
    assert_eq!(
        read(&f, &thread, &terminal, &old, cx).unwrap()["state"],
        "cancelled"
    );
    let new = start(&f, &thread, &terminal, cx);
    assert_ne!(new, old);
    assert_eq!(replacement.requests.lock().unwrap().len(), 1);
    assert!(!replacement.closed(0));
}

#[gpui_kit::test]
fn disabling_ai_cancels_execution_and_revokes_chat_access(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    start(&f, &thread, &terminal, cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
    })
    .detach();
    cx.run_until_parked();
    assert!(programs.closed(0));
    cx.update(|cx| assert!(thread.read(cx).registration().is_none()));
}

#[gpui_kit::test]
fn deleting_and_releasing_a_chat_drops_its_program_output(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    start(&f, &thread, &terminal, cx);
    let id = thread.entity_id();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.delete_thread(id, window, cx));
    })
    .unwrap();
    drop(thread);
    cx.run_until_parked();
    assert!(programs.closed(0));
}

#[gpui_kit::test]
fn an_absolute_deadline_ends_a_silent_command_with_unknown_exit(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    let mut command = request(&terminal);
    command.timeout_ms = Some(50);
    let payload = result(submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(command),
        true,
        cx,
    ))
    .unwrap();
    let id = payload["command_id"].as_str().unwrap();
    cx.executor().advance_clock(Duration::from_millis(75));
    cx.run_until_parked();
    assert!(programs.closed(0));
    let payload = read(&f, &thread, &terminal, id, cx).unwrap();
    assert_eq!(payload["state"], "timed_out");
    assert!(payload["exit_status"].is_null());
}

#[gpui_kit::test]
fn a_joined_reconnect_command_does_not_hold_input_until_its_process_exits(cx: &mut TestAppContext) {
    let (f, thread, programs, terminal) = setup(cx);
    f.access.link().remote().set(true);
    f.access.link().closed().set(true);
    let command = submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(request(&terminal)),
        true,
        cx,
    );
    let input = submit(
        &f,
        &thread,
        TerminalCall::SendInput(nocterm_ai::SendInput {
            terminal_id: terminal.clone(),
            text: "following input".into(),
            press_enter: true,
        }),
        true,
        cx,
    );
    assert_eq!(f.access.link().reconnects().get(), 1);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let command = result(command).unwrap();
    assert!(matches!(
        command["state"].as_str(),
        Some("starting" | "running")
    ));
    assert!(result(input).is_ok());
    assert_eq!(*f.access.sent().borrow(), ["following input\r"]);
    assert_eq!(programs.requests.lock().unwrap().len(), 1);
    assert!(!programs.closed(0), "the first command is still running");
    programs.finish(0, b"finished", 0, "");
    cx.run_until_parked();
}
mod screenshot_history;
