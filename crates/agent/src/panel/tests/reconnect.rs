//! A server terminal whose connection dropped reconnects when the agent
//! uses it, instead of the agent asking the user to attach it again.
use super::*;
use std::time::Duration;

fn read(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> Result<serde_json::Value, nocterm_ai::ToolFailure> {
    let (respond, mut response) = oneshot::channel();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            let registration_id = thread.registration().as_ref().unwrap().id;
            let terminal_id = thread.resolved(cx)[0].0.clone();
            f.calls
                .try_send(BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id,
                    call: nocterm_ai::TerminalCall::ReadTerminal(nocterm_ai::ReadTerminal {
                        terminal_id,
                        lines: None,
                        since: None,
                    }),
                    respond,
                })
                .unwrap();
        })
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        if thread.read(cx).has_pending_approvals() {
            window.click(("tool-Allow once", 0usize), cx);
        }
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    response.try_recv().unwrap().expect("answered")
}

#[gpui_kit::test]
fn a_dropped_server_terminal_reconnects_when_the_agent_reads_it(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.link().remote().set(true);
    f.access.link().closed().set(true);
    let answer = read(&f, cx).expect("read after reconnecting");
    assert_eq!(answer["text"], "output");
    assert_eq!(f.access.link().reconnects().get(), 1);
    assert!(!f.access.link().closed().get());

    // A connected terminal is used as it is.
    read(&f, cx).unwrap();
    assert_eq!(f.access.link().reconnects().get(), 1);
}

#[gpui_kit::test]
fn an_unreachable_server_is_reported_and_its_tab_stays(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.link().remote().set(true);
    f.access.link().closed().set(true);
    f.access.link().unreachable().set(true);
    let error = read(&f, cx).unwrap_err();
    assert_eq!(
        error.message(),
        "Could not connect to the server. Tell the user right away, in one short sentence, which terminal or server you could not reach and why. Do not try to connect another way: nocterm holds the credentials."
    );
    assert_eq!(f.access.link().reconnects().get(), 1);
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
    f.access.link().closed().set(true);
    let _ = read(&f, cx);
    assert_eq!(f.access.link().reconnects().get(), 0);
}

#[gpui_kit::test]
fn identical_secret_prompt_tokens_are_specific_to_the_original_request(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.access.replace_prompt();
    let original = cx.update(|cx| f.access.pending(cx).unwrap());
    f.access.replace_prompt();
    let current = cx.update(|cx| f.access.pending(cx).unwrap());
    assert_eq!(original.label, current.label);
    assert_ne!(original.token, current.token);
    cx.update(|cx| {
        assert!(matches!(
            f.access
                .answer(original.token, nocterm_session::Secret::new("stale"), cx),
            Err(nocterm_workspace::TerminalError::StalePrompt)
        ));
        f.access
            .answer(current.token, nocterm_session::Secret::new("current"), cx)
            .unwrap();
    });
    assert_eq!(*f.access.sign_in().answers().borrow(), ["current"]);
}
