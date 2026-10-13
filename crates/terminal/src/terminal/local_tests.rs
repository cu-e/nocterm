use super::*;
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_session::Command;
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
            assert_eq!(terminal.spec().title, "Auth Hermes");
            assert_eq!(
                terminal.spec().launch.as_ref().unwrap().args,
                vec!["--setup"]
            );
            assert!(!terminal.recording.options().auto_start);
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
            assert_eq!(
                terminal.change_directory(Path::new("/tmp/a"), cx),
                Err(nocterm_workspace::LocalDirectoryError::Busy {
                    command: "cd -- '/tmp/a'".into(),
                })
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
            assert_eq!(
                terminal.change_directory(Path::new("/tmp/c"), cx),
                Err(nocterm_workspace::LocalDirectoryError::Busy {
                    command: "cd -- '/tmp/c'".into(),
                })
            );
        })
    });
}

fn directory_terminal(
    cx: &mut TestAppContext,
    program: &str,
    charset: nocterm_session::Charset,
) -> (
    Entity<Terminal>,
    Arc<Mutex<Option<nocterm_session::SessionDriver>>>,
) {
    let driver = Arc::new(Mutex::new(None));
    let captured = driver.clone();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_session::LocalShellSettings>(|section| {
            section.program = Some(program.into());
        });
        settings.update::<nocterm_ui::TerminalSettings>(|section| section.charset = charset);
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
    (terminal, driver)
}

#[gpui_kit::test]
fn directory_bridge_validates_connection_and_literal_paths_before_submission(
    cx: &mut TestAppContext,
) {
    use nocterm_workspace::LocalDirectoryError;
    let (terminal, driver) = directory_terminal(cx, "/bin/bash", nocterm_session::Charset::Utf8);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            assert_eq!(
                terminal.change_directory(Path::new("/tmp"), cx),
                Err(LocalDirectoryError::NotConnectedLocal)
            );
            terminal.handle_event(Event::Connected, cx);
            for path in ["/tmp/a\0b", "/tmp/a\rb", "/tmp/a\nb"] {
                assert_eq!(
                    terminal.change_directory(Path::new(path), cx),
                    Err(LocalDirectoryError::ControlCharacters)
                );
            }
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt as _;
                let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
                assert_eq!(
                    terminal.change_directory(path, cx),
                    Err(LocalDirectoryError::NonUtf8Path)
                );
            }
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
            terminal
                .change_directory(Path::new("/tmp/literal"), cx)
                .unwrap();
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert_eq!(
        driver.next_command().now_or_never().unwrap(),
        Some(Command::Input(b"cd -- '/tmp/literal'\r".to_vec()))
    );
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn directory_bridge_refuses_unsupported_shell_without_sending(cx: &mut TestAppContext) {
    let (terminal, driver) = directory_terminal(cx, "cmd.exe", nocterm_session::Charset::Utf8);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
            assert_eq!(
                terminal.change_directory(Path::new("/tmp"), cx),
                Err(nocterm_workspace::LocalDirectoryError::UnsupportedShell)
            );
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn directory_bridge_refuses_alternate_screen_and_accepts_after_leaving(cx: &mut TestAppContext) {
    let (terminal, driver) = directory_terminal(cx, "/bin/bash", nocterm_session::Charset::Utf8);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07\x1b[?1049h".to_vec()), cx);
            assert_eq!(
                terminal.change_directory(Path::new("/tmp"), cx),
                Err(nocterm_workspace::LocalDirectoryError::Busy {
                    command: "cd -- '/tmp'".into(),
                })
            );
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert!(driver.next_command().now_or_never().is_none());
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Output(b"\x1b[?1049l\x1b]133;A\x07".to_vec()), cx);
            terminal.change_directory(Path::new("/tmp"), cx).unwrap();
        })
    });
    assert_eq!(
        driver.next_command().now_or_never().unwrap(),
        Some(Command::Input(b"cd -- '/tmp'\r".to_vec()))
    );
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn directory_bridge_preserves_unrepresentable_charset_cause_without_sending(
    cx: &mut TestAppContext,
) {
    use std::error::Error as _;
    let charset = nocterm_session::Charset::Windows1252;
    let (terminal, driver) = directory_terminal(cx, "/bin/bash", charset);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
            let error = terminal
                .change_directory(Path::new("/tmp/中文"), cx)
                .unwrap_err();
            assert_eq!(
                error,
                nocterm_workspace::LocalDirectoryError::Encoding(
                    nocterm_session::TextEncodingError::new(charset)
                )
            );
            assert_eq!(
                error
                    .source()
                    .unwrap()
                    .downcast_ref::<nocterm_session::TextEncodingError>()
                    .unwrap()
                    .charset(),
                charset
            );
            terminal
                .change_directory(Path::new("/tmp/ascii"), cx)
                .unwrap();
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert_eq!(
        driver.next_command().now_or_never().unwrap(),
        Some(Command::Input(b"cd -- '/tmp/ascii'\r".to_vec()))
    );
    assert!(driver.next_command().now_or_never().is_none());
}

#[gpui_kit::test]
fn directory_bridge_reports_full_input_queue_and_can_retry_after_drain(cx: &mut TestAppContext) {
    let (terminal, driver) = directory_terminal(cx, "/bin/bash", nocterm_session::Charset::Utf8);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Connected, cx);
            assert!(terminal.send(vec![b'x'; 4 * 1024 * 1024]));
            terminal.handle_event(Event::Output(b"\x1b]133;A\x07".to_vec()), cx);
            assert_eq!(
                terminal.change_directory(Path::new("/tmp"), cx),
                Err(nocterm_workspace::LocalDirectoryError::QueueFull)
            );
        })
    });
    let driver = driver.lock().unwrap().take().unwrap();
    assert!(matches!(
        driver.next_command().now_or_never().unwrap(),
        Some(Command::Input(_))
    ));
    assert!(driver.next_command().now_or_never().is_none());
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.change_directory(Path::new("/tmp"), cx).unwrap()
        })
    });
    assert_eq!(
        driver.next_command().now_or_never().unwrap(),
        Some(Command::Input(b"cd -- '/tmp'\r".to_vec()))
    );
}

#[gpui_kit::test]
fn command_completion_effect_observes_closed_state_and_retired_prompt_authority(
    cx: &mut TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
    });
    for disconnect in [false, true] {
        let weak: Rc<RefCell<Option<gpui_kit::WeakEntity<Terminal>>>> = Rc::default();
        let observed = Rc::new(RefCell::new(Vec::new()));
        let callback_owner = weak.clone();
        let callback_observed = observed.clone();
        let terminal = cx.update(|cx| {
            cx.new(|cx| {
                Terminal::new_local_command(
                    nocterm_session::ShellLaunch::default(),
                    "Completion state".into(),
                    Arc::new(FakeTransport(Arc::new(Mutex::new(None)))),
                    move |reason, cx| {
                        let terminal = callback_owner.borrow().as_ref().unwrap().upgrade().unwrap();
                        let observed = callback_observed.clone();
                        // GPUI effects observe entities after the current model update
                        // releases its borrow; the completion contract is synchronous.
                        cx.defer(move |cx| {
                            let terminal = terminal.read(cx);
                            observed.borrow_mut().push((
                                reason,
                                terminal.status().clone(),
                                terminal.connection_epoch(),
                                terminal.prompt().is_none(),
                            ));
                        });
                    },
                    cx,
                )
            })
        });
        weak.replace(Some(terminal.downgrade()));
        let epoch = cx.update(|cx| terminal.read(cx).connection_epoch());
        let reason = if disconnect {
            CloseReason::ClosedByUser
        } else {
            CloseReason::Exited(Some(0))
        };
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                let (reply, _) = nocterm_session::Reply::channel();
                terminal.handle_event(
                    Event::Prompt(Prompt::Secret {
                        request: nocterm_session::SecretRequest::Interactive {
                            prompt: "Pending".into(),
                            echo: false,
                        },
                        reply,
                    }),
                    cx,
                );
                if disconnect {
                    terminal.disconnect(cx);
                } else {
                    terminal.handle_event(Event::Closed(reason.clone()), cx);
                }
            })
        });
        assert_eq!(
            *observed.borrow(),
            vec![(
                reason.clone(),
                Status::Closed(reason),
                epoch + u64::from(disconnect),
                true,
            )]
        );
        cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.disconnect(cx)));
        assert_eq!(
            observed.borrow().len(),
            1,
            "completion is taken exactly once"
        );
    }
}
