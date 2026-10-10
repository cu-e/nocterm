//! A server terminal whose connection dropped reconnects when the agent
//! uses it, instead of the agent asking the user to attach it again.
use super::*;
use std::time::Duration;

fn read(f: &Fixture, cx: &mut TestAppContext) -> Result<serde_json::Value, String> {
    let (respond, mut response) = oneshot::channel();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            let registration_id = thread.registration().as_ref().unwrap().id;
            let terminal_id = thread.resolved(cx)[0].0.clone();
            thread.handle_tool(
                BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id,
                    call: nocterm_ai::TerminalCall::ReadTerminal(nocterm_ai::ReadTerminal {
                        terminal_id,
                        lines: None,
                        since: None,
                    }),
                    respond,
                },
                cx,
            );
            if !thread.tools.is_empty() {
                thread.approve_tool(0, true, false, cx);
            }
        })
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    response.try_recv().unwrap().expect("answered")
}

#[gpui_kit::test]
fn a_dropped_server_terminal_reconnects_when_the_agent_reads_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.link.remote.set(true);
    f.access.link.closed.set(true);
    let answer = read(&f, cx).expect("read after reconnecting");
    assert_eq!(answer["text"], "output");
    assert_eq!(f.access.link.reconnects.get(), 1);
    assert!(!f.access.link.closed.get());

    // A connected terminal is used as it is.
    read(&f, cx).unwrap();
    assert_eq!(f.access.link.reconnects.get(), 1);
}

#[gpui_kit::test]
fn an_unreachable_server_is_reported_and_its_tab_stays(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.link.remote.set(true);
    f.access.link.closed.set(true);
    f.access.link.unreachable.set(true);
    let error = read(&f, cx).unwrap_err();
    assert!(error.contains("Could not connect"), "{error}");
    assert_eq!(f.access.link.reconnects.get(), 1);
    cx.update(|cx| {
        assert!(
            f.workspace
                .read(cx)
                .items()
                .any(|item| item.item_id() == f.terminal)
        );
    });
}

#[gpui_kit::test]
fn a_closed_local_shell_is_not_restarted_behind_the_user(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.link.closed.set(true);
    let _ = read(&f, cx);
    assert_eq!(f.access.link.reconnects.get(), 0);
}
