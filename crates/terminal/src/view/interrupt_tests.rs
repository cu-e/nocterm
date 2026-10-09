//! Interrupt keys pass through the rooted window's real keymap and dispatch tree.
use super::*;

fn select(view: &Entity<TerminalView>, cx: &mut TestAppContext) {
    view.update(cx, |v, cx| {
        v.terminal.update(cx, |t, cx| {
            t.update_emulator(cx, |e| {
                e.start_selection(
                    SelectionKind::Cells,
                    CellPoint { row: 0, col: 0 },
                    nocterm_vt::Side::Left,
                );
                e.update_selection(CellPoint { row: 0, col: 5 }, nocterm_vt::Side::Right);
            });
        });
    });
}

#[gpui_kit::test]
fn ctrl_c_without_selection_interrupts_without_copying(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target".to_vec()));
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(drain(&driver), [vec![3]]);
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("unchanged")
        )
    });
}

#[gpui_kit::test]
fn ctrl_c_respects_negotiated_keyboard_protocol(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"\x1b[>3u".to_vec()));
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [b"\x1b[99;5u".to_vec(), b"\x1b[99;5:3u".to_vec()]
    );
}

#[gpui_kit::test]
fn menu_copy_and_search_ctrl_c_still_copy_without_shell_input(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target".to_vec()));
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    select(&view, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(native_input::Copy), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            view.read(cx)
                .terminal
                .read(cx)
                .emulator()
                .selection_text()
                .as_deref(),
            Some("target")
        );
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("target")
        );
        view.update(cx, |v, cx| v.find(&nocterm_workspace::Find, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("search term", cx);
        window.dispatch_action(Box::new(native_input::SelectAll), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            },
            cx,
        );
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("search term")
        );
    })
    .unwrap();
    assert!(drain(&driver).is_empty());
}

#[gpui_kit::test]
fn ctrl_c_with_selection_interrupts_and_keeps_clipboard_unchanged(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target".to_vec()));
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    select(&view, cx);
    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(drain(&driver), [vec![3]]);
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("unchanged")
        )
    });
}

#[gpui_kit::test]
fn root_dispatch_preserves_control_keys_and_tab_screen_input(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        for key in [
            "ctrl-a",
            "ctrl-e",
            "ctrl-d",
            "ctrl-l",
            "ctrl-s",
            "ctrl-q",
            "ctrl-z",
            "ctrl-space",
            "tab",
            "shift-tab",
        ] {
            window.press(key, cx);
        }
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [
            vec![1],
            vec![5],
            vec![4],
            vec![12],
            vec![19],
            vec![17],
            vec![26],
            vec![0],
            vec![9],
            b"\x1b[Z".to_vec()
        ]
    );
}
