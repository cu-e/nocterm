//! Each chat's terminal tools reach that chat, and agents are told to report
//! a terminal or server they cannot reach.
use super::*;
use nocterm_ai::tool_display::bridge_server_name;
use nocterm_ai::{RunCommand, TerminalCall};

fn server_names(f: &Fixture) -> Vec<String> {
    f.commands
        .servers
        .lock()
        .unwrap()
        .iter()
        .map(|servers| {
            assert_eq!(servers.len(), 1);
            match &servers[0] {
                acp::McpServer::Stdio(server) => server.name.clone(),
                other => panic!("unexpected server {other:?}"),
            }
        })
        .collect()
}

fn registration(thread: &Entity<crate::thread::AgentThread>, cx: &App) -> u64 {
    thread.read(cx).registration().as_ref().unwrap().id
}

fn run(
    registration: u64,
    terminal: &str,
) -> (
    BridgeCall,
    oneshot::Receiver<Result<serde_json::Value, String>>,
) {
    let (respond, response) = oneshot::channel();
    (
        BridgeCall {
            arguments: None,
            display_token: None,
            registration_id: registration,
            call: TerminalCall::RunCommand(RunCommand {
                terminal_id: terminal.into(),
                command: "uptime".into(),
                timeout_ms: None,
                idle_ms: None,
            }),
            respond,
        },
        response,
    )
}

#[gpui_kit::test]
fn each_chat_gives_its_agent_a_server_of_its_own(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    // Not a draft, so the second chat does not replace it.
    cx.update(|cx| first.update(cx, |thread, _| thread.name = Some("first".into())));
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    assert_eq!(
        f.connector.connects.load(Ordering::SeqCst),
        2,
        "each chat owns an independent agent process"
    );
    let names = server_names(&f);
    assert_eq!(names.len(), 2);
    assert_ne!(
        names[0], names[1],
        "agents that keep one server per name for the whole process would route both chats to the first"
    );
    cx.update(|cx| {
        assert_eq!(names[0], bridge_server_name(registration(&first, cx)));
        assert_eq!(names[1], bridge_server_name(registration(&second, cx)));
    });
}

#[gpui_kit::test]
fn tool_calls_and_approvals_reach_the_chat_that_owns_the_registration(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| {
        first.update(cx, |thread, cx| {
            thread.name = Some("first".into());
            // The first chat has no terminals.
            thread.attach(Attachment::Terminal(f.terminal), cx);
            assert!(thread.composer.attachments.is_empty());
        })
    });
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (second_id, terminal) = cx.update(|cx| {
        let id = registration(&second, cx);
        let terminal = second.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone());
        (id, terminal)
    });

    let (respond, listed) = oneshot::channel();
    f.calls
        .try_send(BridgeCall {
            arguments: None,
            display_token: None,
            registration_id: second_id,
            call: TerminalCall::ListTerminals,
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    let listed = listed.now_or_never().unwrap().unwrap().unwrap();
    assert!(
        listed["context"].as_str().unwrap().contains(&terminal),
        "the second chat lists its own terminal: {listed}"
    );

    let (call, response) = run(second_id, &terminal);
    f.calls.try_send(call).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            second.read(cx).tools.len(),
            1,
            "the approval waits in the chat that asked"
        );
        assert!(first.read(cx).tools.is_empty());
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("agent-approvals").visible());
        f.panel.update(cx, |panel, cx| {
            panel.active = panel
                .threads
                .iter()
                .position(|thread| thread.entity_id() == first.entity_id());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(
            window.try_find("agent-approvals").is_none(),
            "another chat does not show the request"
        );
    })
    .unwrap();
    cx.update(|cx| second.update(cx, |thread, cx| thread.approve_tool(0, true, false, cx)));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    let answer = response.now_or_never().unwrap().unwrap().unwrap();
    assert_eq!(answer["completion"], "prompt_returned");
    assert_eq!(f.access.sent.borrow().as_slice(), ["uptime"]);
}

#[gpui_kit::test]
fn prompts_tell_the_agent_to_report_unreachable_servers(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "check the server", "ok", cx);
    let prompt = f.commands.prompts.lock().unwrap()[0].clone();
    let acp::ContentBlock::Text(context) = &prompt.prompt[0] else {
        panic!("the prompt starts with its context");
    };
    assert!(
        context
            .text
            .starts_with(nocterm_ai::context::TERMINAL_RULES),
        "{}",
        context.text
    );
    assert!(context.text.contains("<nocterm_context>"));
}

#[gpui_kit::test]
fn an_unreachable_terminal_asks_the_agent_to_tell_the_user(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (call, response) = cx.update(|cx| run(registration(&thread, cx), "t99"));
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.handle_tool(call, cx);
            thread.approve_tool(0, true, false, cx);
        })
    });
    let error = response.now_or_never().unwrap().unwrap().unwrap_err();
    assert!(error.starts_with("Terminal was detached"), "{error}");
    assert!(error.contains("Tell the user right away"), "{error}");
    assert!(f.access.sent.borrow().is_empty());
}
