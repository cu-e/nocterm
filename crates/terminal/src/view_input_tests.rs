use super::*;

fn watch(
    cx: &mut TestAppContext,
    view: &Entity<TerminalView>,
) -> (Rc<Cell<usize>>, Rc<Cell<usize>>, Vec<Subscription>) {
    let notifications = Rc::new(Cell::new(0));
    let outputs = Rc::new(Cell::new(0));
    let subscriptions = cx.update(|cx| {
        let notifications = notifications.clone();
        let outputs = outputs.clone();
        let terminal = view.read(cx).terminal.clone();
        vec![
            cx.observe(view, move |_, _| notifications.set(notifications.get() + 1)),
            cx.subscribe(&terminal, move |_, event, _| {
                if *event == TerminalEvent::Output {
                    outputs.set(outputs.get() + 1);
                }
            }),
        ]
    });
    (notifications, outputs, subscriptions)
}

#[gpui_kit::test]
fn ordinary_typing_waits_for_delayed_echo_without_redrawing(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    let (notifications, outputs, _subscriptions) = watch(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.input("abc", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver).concat(), b"abc\x7f");
    assert_eq!(outputs.get(), 0);
    assert_eq!(notifications.get(), 0);
    view.read_with(cx, |v, _| assert_eq!(v.frame.borrow().row_text(0), ""));
    emit(
        cx,
        &transport,
        0,
        Event::Output("ab界e\u{301}".as_bytes().to_vec()),
    );
    assert_eq!(outputs.get(), 1);
    assert!(notifications.get() > 0);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    view.read_with(cx, |v, _| {
        let frame = v.frame.borrow();
        assert_eq!(frame.row_text(0), "ab界e");
        assert_eq!(frame.combining, [(4, "\u{301}".to_owned())]);
        assert!(!v.frame_dirty.get());
    });
}

#[gpui_kit::test]
fn typing_clears_selection_and_returns_scrollback_before_echo(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output("line\r\n".repeat(100).into_bytes()),
    );
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.update_emulator(cx, |e| {
                    e.scroll(Scroll::Top);
                    e.start_selection(
                        SelectionKind::Cells,
                        CellPoint { row: 0, col: 0 },
                        nocterm_vt::Side::Left,
                    );
                    e.update_selection(CellPoint { row: 0, col: 2 }, nocterm_vt::Side::Right);
                });
            });
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        let e = v.terminal.read(cx).emulator();
        assert!(e.display_offset() > 0);
        assert_eq!(e.selection_text().as_deref(), Some("lin"));
    });
    let (_, outputs, _subscriptions) = watch(cx, &view);
    cx.update_window(handle, |_, window, cx| window.input("x", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(outputs.get(), 1);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    view.read_with(cx, |v, cx| {
        let e = v.terminal.read(cx).emulator();
        assert_eq!(e.display_offset(), 0);
        assert!(e.selection_text().is_none());
        assert_eq!(v.frame.borrow().display_offset, 0);
        assert!(!v.frame.borrow().cells.iter().any(|cell| cell.selected));
    });
    cx.update_window(handle, |_, window, cx| window.input("y", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(outputs.get(), 1);
}

fn advance(cx: &mut TestAppContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
}

fn element(view: &Entity<TerminalView>, cx: &App) -> TerminalElement {
    let v = view.read(cx);
    TerminalElement {
        view: view.clone(),
        terminal: v.terminal.clone(),
        focus: v.focus_handle.clone(),
        style: TerminalStyle::current(cx),
        focused: v.focused,
        cursor_lit: v.cursor_lit,
        marked_text: v.marked_text.clone().map(SharedString::from),
        frame: v.frame.clone(),
        highlights: v.highlights.clone(),
        frame_dirty: v.frame_dirty.clone(),
        frame_version: v.frame_version.clone(),
        geometry: v.geometry.clone(),
    }
}

#[gpui_kit::test]
fn snapshot_cache_tracks_same_turn_selection_scroll_search_resize_and_palette(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output("needle\r\n".repeat(100).into_bytes()),
    );
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let element = cx.update(|cx| element(&view, cx));
    cx.update(|cx| {
        assert!(!element.refresh_frame(cx));
        view.update(cx, |v, cx| v.wake_cursor(cx));
        assert!(
            !element.refresh_frame(cx),
            "cursor reset copied emulator snapshot"
        );
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.update_emulator(cx, |e| {
                    e.scroll(Scroll::Top);
                    e.start_selection(
                        SelectionKind::Cells,
                        CellPoint { row: 0, col: 0 },
                        nocterm_vt::Side::Left,
                    );
                    e.update_selection(CellPoint { row: 0, col: 5 }, nocterm_vt::Side::Right);
                });
            });
        });
        assert!(
            element.refresh_frame(cx),
            "queued event left selection snapshot stale"
        );
        assert!(element.frame.borrow().display_offset > 0);
        assert_eq!(
            element
                .frame
                .borrow()
                .cells
                .iter()
                .filter(|c| c.selected)
                .count(),
            6
        );
        view.update(cx, |v, cx| v.type_text("x", cx));
        assert!(element.refresh_frame(cx));
        assert_eq!(element.frame.borrow().display_offset, 0);
        assert!(!element.frame.borrow().cells.iter().any(|c| c.selected));
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.resize(nocterm_vt::TermSize::new(40, 10, 7, 14), cx)
            });
        });
        assert!(element.refresh_frame(cx));
        assert_eq!(
            element.frame.borrow().size,
            nocterm_vt::TermSize::new(40, 10, 7, 14)
        );
        view.update(cx, |v, cx| {
            let mut style = TerminalStyle::current(cx);
            style.foreground.r ^= 0xff;
            v.sync_palette(&style, cx);
        });
        assert!(element.refresh_frame(cx));
        assert!(!element.refresh_frame(cx));
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                t.begin_find("needle".into(), SearchDirection::Next, None, cx)
            });
        });
    });
    complete(cx, &view);
    cx.update(|cx| {
        element.refresh_frame(cx);
        assert!(element.frame.borrow().cells.iter().any(|c| c.search_hit));
        view.update(cx, |v, cx| v.terminal.update(cx, |t, cx| t.clear_find(cx)));
        assert!(element.refresh_frame(cx));
        assert!(!element.frame.borrow().cells.iter().any(|c| c.search_hit));
    });
}

#[gpui_kit::test]
fn each_key_restarts_cursor_interval_without_notifying_while_lit(cx: &mut TestAppContext) {
    let (handle, view, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    advance(cx, 530);
    assert!(!view.read_with(cx, |v, _| v.cursor_lit));
    let (notifications, _, _subscriptions) = watch(cx, &view);
    cx.update_window(handle, |_, window, cx| window.input("a", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.cursor_lit));
    assert_eq!(notifications.get(), 1);
    advance(cx, 300);
    cx.update_window(handle, |_, window, cx| window.input("b", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1);
    advance(cx, 230);
    assert!(view.read_with(cx, |v, _| v.cursor_lit));
    advance(cx, 299);
    assert!(view.read_with(cx, |v, _| v.cursor_lit));
    advance(cx, 1);
    assert!(!view.read_with(cx, |v, _| v.cursor_lit));
    assert_eq!(notifications.get(), 2);
}

#[gpui_kit::test]
fn composition_commit_and_cancel_redraw_only_when_overlay_changes(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    let (notifications, _, _subscriptions) = watch(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "界", None, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1);
    assert!(drain(&driver).is_empty());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "界", None, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.replace_text_in_range(None, "界", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 2);
    assert_eq!(drain(&driver), ["界".as_bytes().to_vec()]);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.unmark_text(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 2);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "cancel", None, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 3);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| v.unmark_text(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(notifications.get(), 4);
    assert!(drain(&driver).is_empty());
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn scrolling_notifies_and_revises_only_when_display_changes(cx: &mut TestAppContext) {
    let (_, view, transport) = fixture(cx);
    let terminal = view.read_with(cx, |v, _| v.terminal.clone());
    let (notifications, outputs, _subscriptions) = watch(cx, &view);
    let step = |cx: &mut TestAppContext, scroll: Scroll, changed: bool| {
        let revision = terminal.read_with(cx, |t, _| t.display_revision());
        let before = outputs.get();
        let before_notifications = notifications.get();
        cx.update(|cx| {
            terminal.update(cx, |t, cx| t.scroll(scroll, cx));
            assert_eq!(
                terminal.read(cx).display_revision(),
                revision + u64::from(changed)
            );
        });
        cx.run_until_parked();
        assert_eq!(outputs.get(), before + usize::from(changed));
        if !changed {
            assert_eq!(notifications.get(), before_notifications);
        }
    };
    for scroll in [
        Scroll::Top,
        Scroll::Bottom,
        Scroll::PageUp,
        Scroll::PageDown,
        Scroll::Lines(0),
        Scroll::Lines(1),
        Scroll::Lines(-1),
    ] {
        step(cx, scroll, false);
    }
    emit(
        cx,
        &transport,
        0,
        Event::Output("line\r\n".repeat(100).into_bytes()),
    );
    let history = terminal.read_with(cx, |t, _| {
        let mut frame = Frame::default();
        t.emulator().snapshot(&mut frame);
        frame.history_size
    });
    assert!(history > 0);
    while terminal.read_with(cx, |t, _| t.emulator().display_offset()) < history {
        step(cx, Scroll::PageUp, true);
    }
    cx.update(|cx| {
        terminal.update(cx, |t, cx| {
            t.update_emulator(cx, |e| {
                e.start_selection(
                    SelectionKind::Cells,
                    CellPoint { row: 0, col: 0 },
                    nocterm_vt::Side::Left,
                );
                e.update_selection(CellPoint { row: 0, col: 2 }, nocterm_vt::Side::Right);
            })
        });
    });
    cx.run_until_parked();
    for scroll in [
        Scroll::Top,
        Scroll::PageUp,
        Scroll::Lines(1),
        Scroll::Lines(0),
    ] {
        step(cx, scroll, false);
        assert_eq!(
            terminal
                .read_with(cx, |t, _| t.emulator().selection_text())
                .as_deref(),
            Some("lin")
        );
    }
    while terminal.read_with(cx, |t, _| t.emulator().display_offset()) > 0 {
        step(cx, Scroll::PageDown, true);
    }
    for scroll in [
        Scroll::Bottom,
        Scroll::PageDown,
        Scroll::Lines(-1),
        Scroll::Lines(0),
    ] {
        step(cx, scroll, false);
    }
    step(cx, Scroll::Top, true);
    step(cx, Scroll::Top, false);
    step(cx, Scroll::Bottom, true);
    step(cx, Scroll::Bottom, false);
}
