use super::*;
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_session::{
    Auth, Charset, CloseReason, Command, Event, Prompt, Reply, SessionDriver, Target,
};
use nocterm_workspace::SessionSpec;

fn fixture(cx: &mut TestAppContext, local: bool) -> (Entity<Terminal>, SessionDriver) {
    fixture_with_charset(cx, local, Charset::Utf8)
}
fn fixture_with_charset(
    cx: &mut TestAppContext,
    local: bool,
    charset: Charset,
) -> (Entity<Terminal>, SessionDriver) {
    let (terminal, driver, _) = fixture_with_transport(cx, local, charset);
    (terminal, driver)
}
fn fixture_with_transport(
    cx: &mut TestAppContext,
    local: bool,
    charset: Charset,
) -> (
    Entity<Terminal>,
    SessionDriver,
    Arc<nocterm_session::testing::ScriptedTransport>,
) {
    let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
    let driver = transport.queue(None, None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_ui::TerminalSettings>(|settings| settings.charset = charset);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(settings),
            cx,
        );
        crate::init(transport.clone(), &ui, cx);
        let local_transport = transport.clone();
        crate::init_local(Arc::new(move |_| local_transport.clone()), cx);
        cx.new(|cx| {
            if local {
                Terminal::new_local(cx)
            } else {
                Terminal::new(
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
                )
            }
        })
    });
    assert!(futures::executor::block_on(driver.emit(Event::Connected)));
    cx.run_until_parked();
    (terminal, driver, transport)
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
        assert_ne!(terminal.agent_prompt_state().0, Some(true));
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
    let (terminal, driver, transport) = fixture_with_transport(cx, false, Charset::Windows1251);
    let replacement = transport.queue(None, None);
    terminal.update(cx, |terminal, cx| {
        terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
        assert!(terminal.paste_snippet("echo", true, cx).is_err());
        terminal.reconnect(cx);
        terminal.handle_event(Event::Connected, cx);
        let (reply, _) = Reply::channel();
        terminal.handle_event(Event::Prompt(Prompt::UnknownHostKey {
            host: "host".into(),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "test".into(),
            reply,
        }), cx);
        assert!(terminal.paste_snippet("echo", true, cx).is_err());
        terminal.answer_host_key(nocterm_session::HostKeyDecision::Reject, cx);
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
        let error = terminal.paste_snippet("echo 🙂", true, cx).unwrap_err();
        assert!(matches!(&error, TerminalError::Encoding(source) if source.charset() == Charset::Windows1251));
        assert!(std::error::Error::source(&error).is_some());
    });
    assert_eq!(
        driver.next_command().now_or_never(),
        Some(Some(Command::Close))
    );
    assert!(driver.next_command().now_or_never().flatten().is_none());
    assert!(replacement.next_command().now_or_never().is_none());
}
#[gpui_kit::test]
fn rejected_queue_does_not_submit_a_partial_snippet(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    let accepted = terminal.update(cx, |terminal, cx| {
        let mut accepted = 0;
        while terminal.send_protocol(b"queued".to_vec()) {
            accepted += 1;
        }
        assert!(
            terminal
                .paste_snippet("echo must-not-arrive", true, cx)
                .unwrap_err()
                .to_string()
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
    let (terminal, _driver) = fixture(cx, false);
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
            assert_eq!(terminal.emulator().modes().alt_screen, alternate);
        });
        cx.run_until_parked();
        assert_eq!(
            changes.get(),
            expected_changes,
            "Availability changes only on a real alternate-screen transition."
        );
    }
}

#[gpui_kit::test]
fn explorer_paste_is_atomic_bracketed_and_never_executes_even_in_alternate_screen(
    cx: &mut TestAppContext,
) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        terminal.handle_event(Event::Output(b"\x1b[?1049h\x1b[?2004h".to_vec()), cx);
        terminal.paste_paths("'/a b/資料' '/a'\\''b'", cx).unwrap();
    });
    assert_eq!(
        sent(&driver),
        "\x1b[200~'/a b/資料' '/a'\\''b'\x1b[201~".as_bytes()
    );
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn explorer_paste_reports_queue_and_charset_rejection_without_partial_input(
    cx: &mut TestAppContext,
) {
    let (terminal, driver) = fixture_with_charset(cx, true, Charset::Windows1252);
    terminal.update(cx, |terminal, cx| {
        assert!(terminal.paste_paths("'/資料'", cx).is_err());
    });
    assert!(driver.next_command().now_or_never().is_none());
    terminal.update(cx, |terminal, cx| {
        while terminal.send_protocol(b"queued".to_vec()) {}
        assert!(
            terminal
                .paste_paths("'/valid path'", cx)
                .unwrap_err()
                .to_string()
                .contains("queue")
        );
    });
    while driver.next_command().now_or_never().is_some() {}
}

#[gpui_kit::test]
fn ssh_agent_commands_require_a_known_empty_prompt_and_a_single_owner(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        assert_eq!(terminal.agent_prompt_state().0, None);
        assert!(terminal.begin_live_command("echo one", cx).is_err());
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        assert_eq!(terminal.agent_prompt_state().0, Some(true));
        let lease = terminal.begin_live_command("echo one", cx).unwrap();
        assert_eq!(sent(&driver), b"echo one\r");
        assert!(terminal.begin_live_command("echo two", cx).is_err());
        assert!(terminal.agent_send("echo two", false, cx).is_err());
        // Replies generated by the emulator are not a user intervention.
        assert!(terminal.send_protocol(b"reply".to_vec()));
        assert!(lease.is_active());
        assert_eq!(sent(&driver), b"reply");
        // Accepted human input revokes ownership; a stale Stop must not Ctrl-C it.
        assert!(terminal.send(b"user".to_vec()));
        assert!(!lease.is_active());
        assert_eq!(sent(&driver), b"user");
        lease.cancel();
        assert!(driver.next_command().now_or_never().is_none());
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        assert!(terminal.send(b"unfinished".to_vec()));
        assert_eq!(sent(&driver), b"unfinished");
        assert!(terminal.begin_live_command("echo two", cx).is_err());
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        let lease = terminal.begin_live_command("echo two", cx).unwrap();
        assert_eq!(sent(&driver), b"echo two\r");
        lease.cancel();
        assert_eq!(sent(&driver), &[3]);
    });
}

#[gpui_kit::test]
fn reconnect_revokes_live_lease_and_clears_prompt_evidence(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        let lease = terminal.begin_live_command("sleep 10", cx).unwrap();
        assert_eq!(sent(&driver), b"sleep 10\r");
        terminal.close(cx);
        assert!(!lease.is_active());
        assert_eq!(terminal.agent_prompt_state().0, None);
        lease.cancel();
        assert!(!matches!(
            driver.next_command().now_or_never().flatten(),
            Some(Command::Input(_))
        ));
    });
}

#[gpui_kit::test]
fn typed_agent_refusals_never_submit_control_characters_or_oversized_input(
    cx: &mut TestAppContext,
) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        for text in ["echo\0bad", "echo\nbad", "echo\rbad"] {
            assert!(matches!(
                terminal.agent_send(text, true, cx),
                Err(TerminalError::CommandControlCharacters)
            ));
        }
        assert!(matches!(
            terminal.agent_send(&"x".repeat(16 * 1024 + 1), false, cx),
            Err(TerminalError::InputTooLarge)
        ));
        terminal.advance_output(b"\x1b[?1049h", cx);
        assert!(matches!(
            terminal.begin_live_command("echo one", cx),
            Err(TerminalError::CommandPromptUnavailable)
        ));
    });
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn dropping_live_observation_does_not_interrupt_the_held_session(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    terminal.update(cx, |terminal, cx| {
        terminal.advance_output(b"\x1b]133;A\x07", cx);
        let lease = terminal.begin_live_command("sleep 10", cx).unwrap();
        assert_eq!(sent(&driver), b"sleep 10\r");
        drop(lease);
    });
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn snippet_limit_exceeds_agent_limit_and_both_accept_their_exact_boundary(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    let agent = "x".repeat(16 * 1024);
    let snippet = "x".repeat(256 * 1024);
    terminal.update(cx, |terminal, cx| {
        terminal.agent_send(&agent, false, cx).unwrap();
    });
    assert_eq!(sent(&driver), agent.as_bytes());
    terminal.update(cx, |terminal, cx| {
        terminal.paste_snippet(&snippet, false, cx).unwrap();
        assert!(matches!(
            terminal.agent_send(&snippet, false, cx),
            Err(TerminalError::InputTooLarge)
        ));
        assert!(matches!(
            terminal.paste_snippet(&format!("{snippet}x"), false, cx),
            Err(TerminalError::SnippetTooLarge)
        ));
    });
    assert_eq!(sent(&driver), snippet.as_bytes());
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn refused_live_admission_preserves_empty_prompt_and_allows_retry(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    let queued = terminal.update(cx, |terminal, cx| {
        terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
        let mut queued = 0;
        while terminal.send_protocol(b"queued".to_vec()) {
            queued += 1;
        }
        assert!(matches!(
            terminal.begin_live_command("echo retry", cx),
            Err(TerminalError::InputQueueRejected)
        ));
        assert_eq!(terminal.agent_prompt_state(), (Some(true), false));
        queued
    });
    for _ in 0..queued {
        assert_eq!(sent(&driver), b"queued");
    }
    terminal.update(cx, |terminal, cx| {
        let lease = terminal.begin_live_command("echo retry", cx).unwrap();
        assert!(lease.is_active());
        assert_eq!(sent(&driver), b"echo retry\r");
    });
}

#[gpui_kit::test]
fn refused_user_admission_preserves_live_lease_until_accepted_input(cx: &mut TestAppContext) {
    let (terminal, driver) = fixture(cx, false);
    let lease = terminal.update(cx, |terminal, cx| {
        terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
        terminal.begin_live_command("sleep 10", cx).unwrap()
    });
    assert_eq!(sent(&driver), b"sleep 10\r");
    let queued = terminal.update(cx, |terminal, _| {
        let mut queued = 0;
        while terminal.send_protocol(b"queued".to_vec()) {
            queued += 1;
        }
        assert!(!terminal.send(b"human".to_vec()));
        assert!(lease.is_active());
        queued
    });
    for _ in 0..queued {
        assert_eq!(sent(&driver), b"queued");
    }
    terminal.update(cx, |terminal, _| assert!(terminal.send(b"human".to_vec())));
    assert_eq!(sent(&driver), b"human");
    assert!(!lease.is_active());
}
