use super::*;
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::{path::Path, sync::Mutex};

struct FakeTransport(Arc<Mutex<Option<nocterm_session::SessionDriver>>>);
impl nocterm_session::Transport for FakeTransport {
    fn open(&self, _: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        *self.0.lock().unwrap() = Some(driver);
        session
    }
}

#[gpui_kit::test]
fn command_completion_is_once_and_recording_is_disabled(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let result = Rc::new(RefCell::new(Vec::new()));
    let captured = result.clone();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_session::LoggingOptions>(|section| section.auto_start = true);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(settings),
            cx,
        );
        cx.new(|cx| {
            Terminal::new_local_command(
                nocterm_session::ShellLaunch {
                    program: Some("test-agent".into()),
                    args: vec!["--setup".into()],
                    integration: false,
                    ..Default::default()
                },
                "Auth Hermes".into(),
                Arc::new(FakeTransport(Arc::new(Mutex::new(None)))),
                move |reason, _| captured.borrow_mut().push(reason),
                cx,
            )
        })
    });
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            assert_eq!(terminal.spec.title, "Auth Hermes");
            assert_eq!(terminal.spec.launch.as_ref().unwrap().args, vec!["--setup"]);
            assert!(!terminal.recording_options.auto_start);
            terminal.handle_event(Event::Connected, cx);
            terminal.start_recording(cx);
            assert!(!terminal.is_recording());
            terminal.handle_event(Event::Closed(CloseReason::Exited(Some(0))), cx);
            terminal.disconnect(cx);
        })
    });
    assert_eq!(*result.borrow(), vec![CloseReason::Exited(Some(0))]);
}

#[gpui_kit::test]
fn command_completion_reports_cancel_and_drop_cancels_receiver(cx: &mut TestAppContext) {
    use futures::channel::oneshot;
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
    });
    for disconnect in [true, false] {
        let (send, receive) = oneshot::channel();
        let terminal = cx.update(|cx| {
            cx.new(|cx| {
                Terminal::new_local_command(
                    nocterm_session::ShellLaunch::default(),
                    "Auth".into(),
                    Arc::new(FakeTransport(Arc::new(Mutex::new(None)))),
                    move |reason, _| {
                        let _ = send.send(reason);
                    },
                    cx,
                )
            })
        });
        if disconnect {
            cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.disconnect(cx)));
            assert_eq!(
                receive.now_or_never().unwrap().unwrap(),
                CloseReason::ClosedByUser
            );
        } else {
            cx.update(|_| drop(terminal));
            cx.run_until_parked();
            assert!(receive.now_or_never().unwrap().is_err());
        }
    }
}

#[gpui_kit::test]
fn directory_bridge_rejects_busy_input_and_uses_running_shell_snapshot(cx: &mut TestAppContext) {
    let driver = Arc::new(Mutex::new(None));
    let captured = driver.clone();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_session::LocalShellSettings>(|section| {
            section.program = Some("/bin/bash".into())
        });
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(settings),
            cx,
        );
        crate::init_local(
            Arc::new(move |_| Arc::new(FakeTransport(captured.clone()))),
            cx,
        );
        cx.new(Terminal::new_local)
    });
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            assert!(
                terminal
                    .change_directory(Path::new("/tmp/a"), cx)
                    .unwrap_err()
                    .contains("busy")
            );
            terminal.handle_event(
                Event::Output(b"\x1b]7;file://localhost/home/egor\x07\x1b]133;A\x07".to_vec()),
                cx,
            );
            assert_eq!(terminal.cwd(), Some(PathBuf::from("/home/egor")));
            cx.update_setting::<nocterm_session::LocalShellSettings>(|s| {
                s.program = Some("pwsh".into())
            })
            .now_or_never()
            .unwrap()
            .unwrap();
            terminal
                .change_directory(Path::new("/tmp/a' $(id)"), cx)
                .unwrap();
            assert!(
                terminal.change_directory(Path::new("/tmp/b"), cx).is_err(),
                "must wait for the next prompt"
            );
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert_eq!(
        futures::executor::block_on(driver.next_command()),
        Some(nocterm_session::Command::Input(
            b"cd -- '/tmp/a'\\'' $(id)'\r".to_vec()
        ))
    );
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
            terminal.send(b"unfinished".to_vec());
            assert!(
                terminal
                    .change_directory(Path::new("/tmp/c"), cx)
                    .unwrap_err()
                    .contains("unfinished")
            );
        })
    });
}
