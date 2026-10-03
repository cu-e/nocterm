use super::*;
use futures::FutureExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_session::{ConnectRequest, Event, Session, SessionDriver, Transport};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Scripted {
    drivers: Mutex<Vec<Arc<SessionDriver>>>,
    requests: Mutex<Vec<ConnectRequest>>,
}
struct Files;
impl nocterm_session::RemoteFs for Files {
    fn home(&self) -> nocterm_session::FsFuture<String> {
        Box::pin(async { Ok("/".into()) })
    }
    fn read_dir(&self, _: &str) -> nocterm_session::FsFuture<Vec<nocterm_session::DirEntry>> {
        Box::pin(async { Ok(vec![]) })
    }
}
impl Transport for Scripted {
    fn open(&self, request: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(Some(Arc::new(Files)));
        self.requests.lock().unwrap().push(request);
        self.drivers.lock().unwrap().push(Arc::new(driver));
        session
    }
}

fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TerminalView>, Arc<Scripted>) {
    let transport = Arc::new(Scripted::default());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(transport.clone(), cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                TerminalView::new(
                    SessionSpec {
                        title: "test".into(),
                        target: nocterm_session::Target::new("test", "host", 22),
                        auth: nocterm_session::Auth::Password,
                        options: Default::default(),
                        launch: None,
                        credential: None,
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.focus(&view.read(cx).focus_handle(cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    emit(cx, &transport, 0, Event::Connected);
    (handle, view, transport)
}
fn emit(cx: &mut TestAppContext, transport: &Scripted, index: usize, event: Event) {
    let driver = transport.drivers.lock().unwrap()[index].clone();
    cx.background_executor
        .spawn(async move {
            let _ = driver.emit(event).await;
        })
        .detach();
    cx.run_until_parked();
}
fn complete(cx: &mut TestAppContext, view: &Entity<TerminalView>) {
    for _ in 0..500 {
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        if view.read_with(cx, |v, cx| !v.terminal.read(cx).find().searching) {
            return;
        }
    }
    panic!("search did not finish after output became quiet");
}
fn drain(driver: &SessionDriver) -> Vec<Vec<u8>> {
    let mut inputs = vec![];
    while let Some(Some(command)) = driver.next_command().now_or_never() {
        if let nocterm_session::Command::Input(bytes) = command {
            inputs.push(bytes);
        }
    }
    inputs
}

#[gpui_kit::test]
fn find_typing_enter_escape_and_native_copy_stay_out_of_the_shell(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target target".to_vec()));
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.execute(ItemCommand::Find, window, cx));
        window.render_frame(cx);
        window.input("target", cx);
    })
    .unwrap();
    cx.run_until_parked();
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.terminal.read(cx).find().result.count, 2);
        assert_eq!(v.terminal.read(cx).find().result.ordinal, 1);
    });
    cx.update_window(handle, |_, window, cx| {
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.terminal.read(cx).find().result.ordinal, 2)
    });
    cx.update_window(handle, |_, window, cx| {
        window.press("shift-enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.terminal.read(cx).find().result.ordinal, 1)
    });
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(native_input::SelectAll), cx);
        window.dispatch_action(Box::new(native_input::Copy), cx);
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("target")
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(view.read(cx).find.is_none());
        assert!(view.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "find text/Enter/Escape were sent to SSH"
    );
}

#[gpui_kit::test]
fn selection_search_and_screen_native_actions_keep_independent_text(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"target target\x1b[?2004h".to_vec()),
    );
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.update_emulator(cx, |e| {
                    e.start_selection(
                        SelectionKind::Cells,
                        CellPoint { row: 0, col: 0 },
                        nocterm_vt::Side::Left,
                    );
                    e.update_selection(CellPoint { row: 0, col: 5 }, nocterm_vt::Side::Right);
                })
            });
            v.execute(ItemCommand::FindNextSelection, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        let t = v.terminal.read(cx);
        assert_eq!(t.find().result.ordinal, 2);
        assert_eq!(t.emulator().selection_text().as_deref(), Some("target"));
        let mut frame = Frame::default();
        t.emulator().snapshot(&mut frame);
        assert_eq!(frame.cells.iter().filter(|c| c.selected).count(), 6);
        assert_eq!(frame.cells.iter().filter(|c| c.search_hit).count(), 6);
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.hide_find(window, cx));
        window.render_frame(cx);
        window.dispatch_action(Box::new(native_input::Copy), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("target")
        );
        cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
    });
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(native_input::Paste), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver), [b"\x1b[200~pasted\x1b[201~".to_vec()]);
}

#[gpui_kit::test]
fn live_reconnect_uses_new_options_and_disconnect_cancels_prompts(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let old = transport.drivers.lock().unwrap()[0].clone();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.set_session_options(
                    nocterm_session::SessionOptions {
                        term: Some("vt100".into()),
                        ..Default::default()
                    },
                    cx,
                )
                .unwrap()
            });
            v.execute(ItemCommand::Reconnect, window, cx);
            assert!(!v.terminal.read(cx).session_context().connected);
        });
    })
    .unwrap();
    assert!(old.closed().now_or_never().is_some());
    assert_eq!(
        transport.requests.lock().unwrap().last().unwrap().term,
        "vt100"
    );
    emit(cx, &transport, 0, Event::Output(b"stale".to_vec()));
    let (reply, answer) = nocterm_session::Reply::channel();
    emit(
        cx,
        &transport,
        1,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "challenge".into(),
                echo: false,
            },
            reply,
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.execute(ItemCommand::Disconnect, window, cx);
            let t = v.terminal.read(cx);
            assert!(t.prompt().is_none());
            assert!(t.session_context().fs.is_none());
            assert!(!t.is_connected());
            assert!(!v.command_enabled(ItemCommand::Disconnect, cx));
            assert!(v.command_enabled(ItemCommand::Reconnect, cx));
            let mut frame = Frame::default();
            t.emulator().snapshot(&mut frame);
            assert!(!frame.cells.iter().any(|c| c.ch == 's'));
        });
    })
    .unwrap();
    assert!(answer.now_or_never().is_some());
    assert!(
        transport.drivers.lock().unwrap()[1]
            .closed()
            .now_or_never()
            .is_some()
    );
}

#[gpui_kit::test]
fn rapid_output_invalidates_highlights_then_search_resumes_and_can_cancel(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"needle".to_vec()));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.open_find(Some("needle".into()), false, window, cx)
        })
    })
    .unwrap();
    complete(cx, &view);
    for _ in 0..4 {
        emit(cx, &transport, 0, Event::Output(b"\r\nneedle".to_vec()));
        view.read_with(cx, |v, cx| {
            let mut frame = Frame::default();
            v.terminal.read(cx).emulator().snapshot(&mut frame);
            assert!(!frame.cells.iter().any(|c| c.search_hit));
        });
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
    }
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.terminal.read(cx).find().result.count, 5)
    });
    emit(cx, &transport, 0, Event::Output(b"\r\nneedle".to_vec()));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.hide_find(window, cx))
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert!(v.find.is_none());
        assert!(v.terminal.read(cx).find().query.is_empty());
        let mut frame = Frame::default();
        v.terminal.read(cx).emulator().snapshot(&mut frame);
        assert!(!frame.cells.iter().any(|c| c.search_hit));
    });
}

#[gpui_kit::test]
fn osc52_requires_opt_in_and_focus_on_the_actual_terminal_screen(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let clipboard = |cx: &mut TestAppContext| {
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
    };
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("original".into())));
    let request = || Event::Output(b"\x1b]52;c;cmVtb3Rl\x07".to_vec());
    emit(cx, &transport, 0, request());
    assert_eq!(clipboard(cx).as_deref(), Some("original"));
    cx.update(|cx| {
        nocterm_ui::update_settings(cx, |s| {
            s.terminal.clipboard_write = nocterm_settings::ClipboardWritePolicy::FocusedTerminal;
        })
        .now_or_never()
        .unwrap()
        .unwrap();
    });
    emit(cx, &transport, 0, request());
    assert_eq!(clipboard(cx).as_deref(), Some("remote"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.execute(ItemCommand::Find, window, cx));
        window.render_frame(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("search".into()));
    })
    .unwrap();
    emit(cx, &transport, 0, request());
    assert_eq!(clipboard(cx).as_deref(), Some("search"));
    cx.update_window(handle, |_, window, cx| {
        window.blur(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("background".into()));
    })
    .unwrap();
    emit(cx, &transport, 0, request());
    assert_eq!(clipboard(cx).as_deref(), Some("background"));
}

#[gpui_kit::test]
fn osc52_cannot_write_from_a_focused_terminal_in_an_inactive_window(cx: &mut TestAppContext) {
    struct OtherWindow;
    impl Render for OtherWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    let (handle, view, transport) = fixture(cx);
    cx.update(|cx| {
        nocterm_ui::update_settings(cx, |settings| {
            settings.terminal.clipboard_write =
                nocterm_settings::ClipboardWritePolicy::FocusedTerminal;
        })
        .now_or_never()
        .unwrap()
        .unwrap();
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            window.activate_window();
            cx.new(|_| OtherWindow)
        })
        .unwrap();
        cx.write_to_clipboard(ClipboardItem::new_string("other window".into()));
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).focus_handle.is_focused(window));
        assert_ne!(cx.active_window(), Some(window.window_handle()));
    })
    .unwrap();
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b]52;c;cmVtb3Rl\x07".to_vec()),
    );
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("other window")
        );
    });
}
