use super::super::Status;
use super::*;
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_session::{
    Auth, Charset, CloseReason, Command, Event, Prompt, Reply, SessionDriver, Target,
};
use nocterm_workspace::SessionSpec;

fn fixture(cx: &mut TestAppContext, local: bool) -> (Entity<Terminal>, SessionDriver) {
    let (session, driver) = nocterm_session::channel(None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        cx.new(|cx| {
            let mut terminal = Terminal::new(
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "test".into(),
                    target: Target::new("me", "host", 22),
                    auth: Auth::Auto,
                    launch: None,
                    credential: None,
                },
                cx,
            );
            terminal.session = Some(session);
            terminal.status = Status::Connected;
            terminal.local = local;
            terminal
        })
    });
    (terminal, driver)
}
fn sent(driver: &SessionDriver) -> Vec<u8> {
    match driver.next_command().now_or_never().flatten().unwrap() {
        Command::Input(bytes) => bytes,
        other => panic!("Unexpected {other:?}"),
    }
}
#[gpui_kit::test]
fn snippet_paste_submits_one_normalized_input_with_one_final_enter(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    for (text, expected) in [
        ("echo one", "echo one\r"),
        ("echo one\n", "echo one\r"),
        ("echo one\r\n\r\n", "echo one\r"),
        ("echo привет\r\necho two\n", "echo привет\recho two\r"),
    ] {
        terminal
            .update(cx, |terminal, cx| terminal.paste_snippet(text, true, cx))
            .unwrap();
        assert_eq!(sent(&driver), expected.as_bytes());
        assert!(
            driver.next_command().now_or_never().is_none(),
            "A run is one atomic submission."
        );
    }
    terminal
        .update(cx, |terminal, cx| {
            terminal.paste_snippet("echo one\n", false, cx)
        })
        .unwrap();
    assert_eq!(sent(&driver), b"echo one\r");
}
#[gpui_kit::test]
fn bracketed_user_paste_works_without_local_integration_and_enters_after_end_marker(
    cx: &mut TestAppContext,
) {
    let (terminal, driver) = fixture(cx, true);
    terminal.update(cx, |terminal, cx| {
        assert!(!terminal.integration.borrow().at_prompt);
        terminal.handle_event(Event::Output(b"\x1b[?2004h".to_vec()), cx);
        terminal
            .paste_snippet("echo привет\r\necho two\n\n", true, cx)
            .unwrap();
        assert!(
            terminal.agent_send("echo guarded", true, cx).is_err(),
            "Agent guards remain intact."
        );
    });
    assert_eq!(
        sent(&driver),
        "\x1b[200~echo привет\r\necho two\x1b[201~\r".as_bytes()
    );
    assert!(driver.next_command().now_or_never().is_none());
}
#[gpui_kit::test]
fn user_paste_rejects_unavailable_authentication_alt_screen_and_invalid_text(
    cx: &mut TestAppContext,
) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        terminal.status = Status::Closed(CloseReason::ClosedByUser);
        assert!(terminal.paste_snippet("echo", true, cx).is_err());
        terminal.status = Status::Connected;
        let (reply, _) = Reply::channel();
        terminal.prompt = Some(Prompt::UnknownHostKey {
            host: "host".into(),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "test".into(),
            reply,
        });
        assert!(terminal.paste_snippet("echo", true, cx).is_err());
        terminal.prompt = None;
        terminal.handle_event(Event::Output(b"\x1b[?1049h".to_vec()), cx);
        assert!(terminal.paste_snippet("echo", true, cx).is_err());
        terminal.handle_event(Event::Output(b"\x1b[?1049l".to_vec()), cx);
        for text in ["", "\n ", "echo\0bad", "echo\x1bbad"] {
            assert!(terminal.paste_snippet(text, true, cx).is_err());
        }
        assert!(
            terminal
                .paste_snippet(&"x".repeat(256 * 1024 + 1), true, cx)
                .is_err()
        );
        terminal.codec = crate::codec::TextCodec::new(Charset::Windows1251);
        assert!(terminal.paste_snippet("echo 🙂", true, cx).is_err());
    });
    assert!(driver.next_command().now_or_never().is_none());
}
#[gpui_kit::test]
fn rejected_queue_does_not_submit_a_partial_snippet(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    let accepted = terminal.update(cx, |terminal, cx| {
        let mut accepted = 0;
        while terminal.session.as_ref().unwrap().input(b"queued".to_vec()) {
            accepted += 1;
        }
        assert!(
            terminal
                .paste_snippet("echo must-not-arrive", true, cx)
                .unwrap_err()
                .contains("queue")
        );
        accepted
    });
    for _ in 0..accepted {
        assert_eq!(sent(&driver), b"queued");
    }
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn alternate_screen_transitions_publish_availability_changes_only_on_mode_flip(
    cx: &mut TestAppContext,
) {
    use std::{cell::Cell, rc::Rc};
    let (terminal, _) = fixture(cx, false);
    let changes = Rc::new(Cell::new(0));
    let observed = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&terminal, move |_, event, _| {
            if matches!(event, TerminalEvent::Changed) {
                observed.set(observed.get() + 1);
            }
        })
    });
    for (bytes, expected_changes, alternate) in [
        (&b"ordinary output"[..], 0, false),
        (&b"\x1b[?1049"[..], 0, false),
        (&b"h"[..], 1, true),
        (&b"more output\x1b[?1049h"[..], 1, true),
        (&b"\x1b[?1049l"[..], 2, false),
        (&b"ordinary output\x1b[?2004h"[..], 2, false),
    ] {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Output(bytes.to_vec()), cx);
            assert_eq!(terminal.emulator.modes().alt_screen, alternate);
        });
        cx.run_until_parked();
        assert_eq!(
            changes.get(),
            expected_changes,
            "Availability changes only on a real alternate-screen transition."
        );
    }
}
