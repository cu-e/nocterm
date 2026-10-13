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
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
        transport.enqueue_session(session);
        let installed = crate::install(
            crate::Deps::new(
                transport,
                crate::LocalSessions::disabled("/bin/bash".into()),
            ),
            &ui,
            cx,
        );
        cx.new(|cx| {
            let mut terminal = Terminal::new(
                installed.clone(),
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
            terminal.handle_event(Event::Connected, cx);
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
fn setup_recording_test(
    directory: &std::path::Path,
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::Entity<Terminal>,
    nocterm_session::Session,
    nocterm_session::SessionDriver,
) {
    let (session, driver) = nocterm_session::channel(None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_session::LoggingOptions>(|section| section.auto_start = true);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(settings),
            cx,
        );
        let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
        transport.enqueue_session(session.clone());
        let log_factory = std::sync::Arc::new(
            nocterm_session::testing::recording::ScriptedOutputLogFactory::default(),
        );
        let log = std::sync::Arc::new(nocterm_session::testing::recording::ScriptedOutputLog::new(
            directory.join("session.log"),
        ));
        log_factory.enqueue(
            nocterm_session::testing::Gate::open(),
            Ok(log.clone() as std::sync::Arc<dyn nocterm_session::recording::OutputLog>),
        );
        log.enqueue_append(
            nocterm_session::testing::Gate::open(),
            nocterm_session::recording::AppendOutcome::new(
                nocterm_session::recording::PhysicalPrefix::new(1, 23, 0, false, false),
                None,
            ),
        );
        log.enqueue_seal(
            nocterm_session::testing::Gate::open(),
            nocterm_session::recording::AppendOutcome::new(
                nocterm_session::recording::PhysicalPrefix::new(1, 23, 0, true, true),
                None,
            ),
        );
        let registry = std::sync::Arc::new(nocterm_store::RecordingRegistry::new(log_factory));
        let installed = crate::install(
            crate::Deps::new(
                transport,
                crate::LocalSessions::disabled("/bin/bash".into()),
            )
            .with_recording(registry),
            &ui,
            cx,
        );
        cx.new(|cx| {
            Terminal::new(
                installed.clone(),
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
    (terminal, session, driver)
}

#[gpui_kit::test]
fn automatic_logs_exclude_authentication_and_input_and_drain_on_disconnect(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let (terminal, _session, driver) = setup_recording_test(directory.path(), cx);
    let (reply, answer) = Reply::channel();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(Prompt::Secret {
                    request: SecretRequest::Password {
                        target: terminal.spec().target.clone(),
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
            terminal.handle_event(Event::Connected, cx);
            terminal.send_text("never-log-input", cx);
            terminal.handle_event(Event::Output("Привет output\n".as_bytes().to_vec()), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
        });
    });
    cx.run_until_parked();
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
    let status = cx.update(|cx| terminal.read(cx).recording_status());
    assert!(status.is_some());
    let status = status.unwrap();
    assert!(status.finished());
    assert!(status.error().is_none());
}
