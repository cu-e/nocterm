use super::*;
use futures::FutureExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_session::{ConnectRequest, Event, Session, SessionDriver, Transport};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Scripted {
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

pub(super) fn fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<TerminalView>, Arc<Scripted>) {
    let transport = Arc::new(Scripted::default());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(transport.clone(), &ui, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                TerminalView::new(
                    SessionSpec {
                        profile: None,
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
pub(super) fn emit(cx: &mut TestAppContext, transport: &Scripted, index: usize, event: Event) {
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
pub(super) fn scripted_driver(transport: &Scripted, index: usize) -> Arc<SessionDriver> {
    transport.drivers.lock().unwrap()[index].clone()
}

pub(super) fn drain(driver: &SessionDriver) -> Vec<Vec<u8>> {
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
        view.update(cx, |v, cx| v.find(&nocterm_workspace::Find, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
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
            v.find_selection(&nocterm_workspace::FindNextSelection, window, cx);
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
            v.reconnect(&Reconnect, window, cx);
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
            v.disconnect(&nocterm_workspace::DisconnectSession, window, cx);
            let t = v.terminal.read(cx);
            assert!(t.prompt().is_none());
            assert!(t.session_context().fs.is_none());
            assert!(!t.is_connected());
            let mut frame = Frame::default();
            t.emulator().snapshot(&mut frame);
            assert!(!frame.cells.iter().any(|c| c.ch == 's'));
        });
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let focus = view.read(cx).focus_handle.clone();
        assert!(!window.is_action_available_in(&nocterm_workspace::DisconnectSession, &focus));
        assert!(window.is_action_available_in(&nocterm_workspace::ReconnectSession, &focus));
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
        cx.update_setting::<nocterm_ui::TerminalSettings>(|s| {
            s.clipboard_write = nocterm_ui::ClipboardWritePolicy::FocusedTerminal;
        })
        .now_or_never()
        .unwrap()
        .unwrap();
    });
    emit(cx, &transport, 0, request());
    assert_eq!(clipboard(cx).as_deref(), Some("remote"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.find(&nocterm_workspace::Find, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
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
        cx.update_setting::<nocterm_ui::TerminalSettings>(|settings| {
            settings.clipboard_write = nocterm_ui::ClipboardWritePolicy::FocusedTerminal;
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

#[gpui_kit::test]
fn find_switches_preserve_query_focus_and_recover_from_invalid_regex(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"Cat cat concatenate".to_vec()),
    );
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.open_find(Some("cat".into()), false, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    complete(cx, &view);
    assert_eq!(
        view.read_with(cx, |v, cx| v.terminal.read(cx).find().result.count),
        2
    );
    for (button, count) in [
        ("find-case-sensitive", 3),
        ("find-whole-word", 2),
        ("find-regex", 2),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(button, cx);
            let input = &view.read(cx).find.as_ref().unwrap().input;
            assert!(input.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(input.read(cx).value().as_ref(), "cat");
        })
        .unwrap();
        complete(cx, &view);
        assert_eq!(
            view.read_with(cx, |v, cx| v.terminal.read(cx).find().result.count),
            count
        );
    }
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(native_input::SelectAll), cx);
        window.input("[", cx);
    })
    .unwrap();
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        let find = v.terminal.read(cx).find();
        assert!(
            find.error
                .as_ref()
                .unwrap()
                .contains("Invalid or oversized")
        );
        assert_eq!(find.result.count, 0);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("find-regex", cx);
    })
    .unwrap();
    complete(cx, &view);
    assert!(view.read_with(cx, |v, cx| v.terminal.read(cx).find().error.is_none()));
    assert!(
        drain(&driver).is_empty(),
        "search switches leaked input to shell"
    );
}

#[gpui_kit::test]
fn find_modes_survive_live_output_invalidation_resize_and_throttled_refresh(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"ID1 id2 xid3 ".to_vec()));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.open_find(Some(r"id\d+".into()), false, window, cx);
            v.terminal.update(cx, |t, cx| {
                t.set_find_options(
                    nocterm_vt::SearchOptions {
                        regex: true,
                        case_sensitive: false,
                        whole_word: true,
                    },
                    cx,
                )
            });
        });
    })
    .unwrap();
    // Invalidate a scan before its background work finishes.
    emit(cx, &transport, 0, Event::Output(b"ID4 ".to_vec()));
    complete(cx, &view);
    assert_eq!(
        view.read_with(cx, |v, cx| v.terminal.read(cx).find().result.count),
        3
    );
    // Completed scans restart after the output throttle with the same options.
    emit(cx, &transport, 0, Event::Output(b"id5 ".to_vec()));
    complete(cx, &view);
    assert_eq!(
        view.read_with(cx, |v, cx| v.terminal.read(cx).find().result.count),
        4
    );
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.resize(nocterm_vt::TermSize::new(9, 4, 0, 0), cx)
            });
        })
    });
    complete(cx, &view);
    view.read_with(cx, |v, cx| {
        let find = v.terminal.read(cx).find();
        assert_eq!(find.result.count, 4);
        assert!(find.options.regex && !find.options.case_sensitive && find.options.whole_word);
    });
}

#[gpui_kit::test]
fn operational_notices_are_event_driven_deduplicated_and_clear_credential_state(
    cx: &mut TestAppContext,
) {
    let (handle, view, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, _| {
                t.credentials.message =
                    Some("Credential was not saved; unlock the vault and retry.".into())
            });
        });
        window.render_frame(cx);
        assert!(
            nocterm_ui::notice::count(window, cx) == 0,
            "render posted an operational notice"
        );
        view.update(cx, |v, cx| {
            v.terminal
                .update(cx, |_, cx| cx.emit(TerminalEvent::Changed));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        window.render_frame(cx);
        assert!(nocterm_ui::notice::run_action(
            window,
            cx,
            "terminal-credential"
        ));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, cx| {
        v.terminal.read(cx).credential_message().is_none()
    }));
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        nocterm_ui::notice::clear(window, cx);
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| t.start_recording(cx))
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        nocterm_ui::notice::clear(window, cx);
        view.update(cx, |v, cx| {
            v.terminal
                .update(cx, |_, cx| cx.emit(TerminalEvent::Output))
        });
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            nocterm_ui::notice::count(window, cx) == 0,
            "unchanged error returned after dismissal"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn semantic_highlighting_updates_open_tabs_without_changing_terminal_text(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"Failed password from 192.0.2.1".to_vec()),
    );
    let assert_role = |expected, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let view = view.read(cx);
            let frame = view.frame.borrow();
            assert_eq!(frame.row_text(0), "Failed password from 192.0.2.1");
            assert_eq!(
                view.highlights.borrow().role_at(0, &frame.cells[0]),
                expected
            );
        })
        .unwrap();
    };
    assert_role(Some(crate::highlighting::Role::Error), cx);
    let scans = view.read_with(cx, |view, _| view.highlights.borrow().scan_count());
    cx.update(|cx| {
        cx.update_setting::<nocterm_ui::AppearanceSettings>(|s| {
            s.mode = nocterm_ui::AppearanceMode::Light;
            s.detect_server_country = false;
        })
        .detach()
    });
    cx.run_until_parked();
    assert_role(Some(crate::highlighting::Role::Error), cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.highlights.borrow().scan_count()),
        scans
    );

    cx.update(|cx| {
        cx.update_setting::<nocterm_ui::TerminalSettings>(|s| s.semantic_highlighting = false)
            .detach()
    });
    cx.run_until_parked();
    assert_role(None, cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ui::TerminalSettings>(|s| s.semantic_highlighting = true)
            .detach()
    });
    cx.run_until_parked();
    assert_role(Some(crate::highlighting::Role::Error), cx);
}

#[path = "view/keyboard_tests.rs"]
mod keyboard_tests;

#[path = "view/keyboard_transition_tests.rs"]
mod keyboard_transition_tests;

#[path = "view_input_tests.rs"]
mod input;

#[path = "view/lease_input_tests.rs"]
mod lease_input_tests;

#[path = "view/pointer_tests.rs"]
mod pointer_tests;

#[path = "view/interrupt_tests.rs"]
mod interrupt_tests;
