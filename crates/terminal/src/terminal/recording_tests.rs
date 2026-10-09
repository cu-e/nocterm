use super::*;
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_session::{Auth, Command, Reply, SecretRequest, Target};
#[gpui_kit::test]
fn oversized_bracketed_paste_is_rejected_without_a_partial_protocol_sequence(
    cx: &mut TestAppContext,
) {
    use futures::FutureExt as _;
    let (session, driver) = nocterm_session::channel(None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        cx.new(|cx| {
            let mut terminal = Terminal::new(
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "test".into(),
                    target: Target::new("me", "host", 22),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                },
                cx,
            );
            terminal.session = Some(session);
            terminal.status = Status::Connected;
            terminal
        })
    });
    let paste = format!("\x1b[200~{}\x1b[201~", "x".repeat(4 * 1024 * 1024));
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.send_text(&paste, cx);
            assert!(terminal.text_error().unwrap().contains("4 MiB"));
        });
    });
    assert!(driver.next_command().now_or_never().is_none());
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.send_text("\x1b[200~small\x1b[201~", cx);
            assert!(terminal.text_error().is_none());
        })
    });
    assert_eq!(
        futures::executor::block_on(driver.next_command()),
        Some(Command::Input(b"\x1b[200~small\x1b[201~".to_vec()))
    );
}
#[gpui_kit::test]
fn automatic_logs_exclude_authentication_and_input_and_drain_on_disconnect(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (session, driver) = nocterm_session::channel(None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_settings::LoggingOptions>(|section| section.auto_start = true);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(settings),
            cx,
        );
        crate::init_recording(directory.path().into(), cx);
        cx.new(|cx| {
            Terminal::new(
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "test".into(),
                    target: Target::new("me", "host", 22),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                },
                cx,
            )
        })
    });
    let (reply, answer) = Reply::channel();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(Prompt::Secret {
                    request: SecretRequest::Password {
                        target: terminal.spec.target.clone(),
                        retry: false,
                    },
                    reply,
                }),
                cx,
            );
            assert!(
                terminal.recording_status().is_none(),
                "do not open logs during authentication"
            );
            terminal.answer_secret(Some(Secret::new("never-log-auth-password")), cx);
            terminal.session = Some(session);
            terminal.handle_event(Event::Connected, cx);
            terminal.send_text("never-log-input", cx);
            terminal.handle_event(Event::Output("Привет output\n".as_bytes().to_vec()), cx);
            terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
        })
    });
    assert_eq!(
        futures::executor::block_on(answer)
            .unwrap()
            .unwrap()
            .expose(),
        "never-log-auth-password"
    );
    assert_eq!(
        futures::executor::block_on(driver.next_command()),
        Some(Command::Input(b"never-log-input".to_vec()))
    );
    for _ in 0..200 {
        if cx.update(|cx| terminal.read(cx).recording_status().unwrap().finished) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = cx.update(|cx| terminal.read(cx).recording_status().unwrap());
    assert!(status.finished);
    assert!(status.error.is_none());
    assert_eq!(
        std::fs::read(status.path.unwrap()).unwrap(),
        "Привет output\n".as_bytes()
    );
}
