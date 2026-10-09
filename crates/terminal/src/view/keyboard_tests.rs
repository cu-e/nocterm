//! All input below is delivered to GPUI's mock TestAppContext window only.
use super::*;
use gpui_kit::{InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke};

fn down(window: &mut Window, key: &str, held: bool, prefer_text: bool, cx: &mut App) {
    let mut keystroke = Keystroke::parse(key).unwrap();
    if keystroke.key.chars().count() == 1 {
        keystroke.key_char = Some(keystroke.key.clone());
    }
    window.dispatch_event(
        KeyDownEvent {
            keystroke,
            is_held: held,
            prefer_character_input: prefer_text,
        }
        .to_platform_input(),
        cx,
    );
}
fn up(window: &mut Window, key: &str, cx: &mut App) {
    window.dispatch_event(
        KeyUpEvent {
            keystroke: Keystroke::parse(key).unwrap(),
        }
        .to_platform_input(),
        cx,
    );
}

#[gpui_kit::test]
fn codex_negotiation_then_shift_enter_text_events_and_legacy_restore(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    emit(cx, &transport, 0, Event::Output(b"\x1b[?u\x1b[c".to_vec()));
    let probe = drain(&driver);
    assert_eq!(probe[0], b"\x1b[?0u");
    assert!(probe[1].ends_with(b"c"));
    emit(cx, &transport, 0, Event::Output(b"\x1b[>7u".to_vec()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("shift-enter", cx);
        window.press("enter", cx);
        window.press("a", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [
            b"\x1b[13;2u".to_vec(),
            b"\r".to_vec(),
            b"a".to_vec(),
            b"\x1b[97;1:3u".to_vec()
        ]
    );
    emit(cx, &transport, 0, Event::Output(b"\x1b[<u".to_vec()));
    cx.update_window(handle, |_, window, cx| window.press("shift-enter", cx))
        .unwrap();
    assert_eq!(drain(&driver), [b"\r".to_vec()]);
}

#[gpui_kit::test]
fn repeat_and_release_preserve_selection_and_scroll_and_drop_orphans(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[>3uone\r\ntwo".to_vec()),
    );
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        up(window, "up", cx);
        assert!(drain(&driver).is_empty(), "unknown releases are ignored");
        down(window, "up", false, false, cx);
        down(window, "up", true, false, cx);
        view.update(cx, |view, cx| {
            view.terminal.update(cx, |terminal, cx| {
                terminal.update_emulator(cx, |e| {
                    e.advance(&b"line\r\n".repeat(100));
                    e.scroll(Scroll::Top);
                    e.start_selection(
                        SelectionKind::Words,
                        CellPoint { row: 0, col: 0 },
                        nocterm_vt::Side::Left,
                    );
                })
            })
        });
        let selection = view.read(cx).terminal.read(cx).emulator().selection_text();
        let offset = view.read(cx).terminal.read(cx).emulator().display_offset();
        assert!(offset > 0);
        up(window, "up", cx);
        assert_eq!(
            view.read(cx).terminal.read(cx).emulator().selection_text(),
            selection
        );
        assert_eq!(
            view.read(cx).terminal.read(cx).emulator().display_offset(),
            offset
        );
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [
            b"\x1b[A".to_vec(),
            b"\x1b[1;1:2A".to_vec(),
            b"\x1b[1;1:3A".to_vec()
        ]
    );
    cx.update_window(handle, |_, window, cx| down(window, "up", false, false, cx))
        .unwrap();
    drain(&driver);
    emit(cx, &transport, 0, Event::Output(b"\x1b[=31u".to_vec()));
    cx.update_window(handle, |_, window, cx| up(window, "up", cx))
        .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "negotiation changes discard earlier ownership"
    );
    cx.update_window(handle, |_, window, cx| {
        down(window, "up", false, false, cx);
        view.update(cx, |view, cx| view.focus_changed(false, cx));
        view.update(cx, |view, cx| view.focus_changed(true, cx));
        up(window, "up", cx);
    })
    .unwrap();
    assert_eq!(drain(&driver), [b"\x1b[A".to_vec()]);
}

#[gpui_kit::test]
fn report_all_text_once_ime_commits_and_altgr_do_not_fabricate_physical_keys(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(cx, &transport, 0, Event::Output(b"\x1b[>31u".to_vec()));
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("a", cx);
        down(window, "ctrl-alt-e", false, true, cx);
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "€", window, cx)
        });
        up(window, "ctrl-alt-e", cx);
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "日", None, window, cx)
        });
        down(window, "enter", false, false, cx);
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "日e\u{301}", window, cx)
        });
        up(window, "enter", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [
            b"\x1b[97;1;97u".to_vec(),
            b"\x1b[97;1:3u".to_vec(),
            b"\x1b[0;;8364u".to_vec(),
            b"\x1b[0;;26085:101:769u".to_vec()
        ]
    );
}

#[gpui_kit::test]
fn app_bindings_platform_keys_and_rejected_presses_never_create_releases(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(cx, &transport, 0, Event::Output(b"\x1b[>31u".to_vec()));
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "ctrl-shift-c",
            Copy,
            Some(KEY_CONTEXT),
        )]);
        window.render_frame(cx);
        down(window, "c", false, false, cx);
        assert_eq!(drain(&driver), [b"\x1b[99;1;99u".to_vec()]);
        window.press("ctrl-shift-c", cx);
        window.press("super-c", cx);
        assert!(drain(&driver).is_empty());
        let terminal = view.read(cx).terminal.clone();
        let mut accepted = 0;
        while terminal.read(cx).send(b"filler".to_vec()) {
            accepted += 1;
            assert!(accepted < 10000);
        }
        down(window, "up", false, false, cx);
        assert_eq!(drain(&driver).len(), accepted);
        up(window, "up", cx);
        assert!(
            drain(&driver).is_empty(),
            "rejected press must not own a release"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn xterm_negotiation_disambiguates_modified_keys_and_restores_legacy(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[>4;2m\x1b[?4m".to_vec()),
    );
    assert_eq!(drain(&driver), [b"\x1b[>4;2m".to_vec()]);
    cx.update_window(handle, |_, window, cx| {
        window.press("shift-enter", cx);
        window.press("shift-tab", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [
            b"\x1b[27;2;13~".to_vec(),
            b"\x1b[Z".to_vec(),
            b"\r".to_vec()
        ]
    );
    emit(cx, &transport, 0, Event::Output(b"\x1b[>4;0m".to_vec()));
    cx.update_window(handle, |_, window, cx| window.press("shift-enter", cx))
        .unwrap();
    assert_eq!(drain(&driver), [b"\r".to_vec()]);
}

#[gpui_kit::test]
fn a_rejected_text_commit_does_not_send_a_release(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ui::TerminalSettings>(|settings| {
            settings.charset = nocterm_session::Charset::Windows1251
        })
        .detach()
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.reconnect(&Reconnect, window, cx))
    })
    .unwrap();
    emit(cx, &transport, 1, Event::Connected);
    emit(cx, &transport, 1, Event::Output(b"\x1b[>7u".to_vec()));
    let driver = transport.drivers.lock().unwrap()[1].clone();
    drain(&driver);
    cx.update_window(handle, |_, window, cx| window.press("日", cx))
        .unwrap();
    assert!(drain(&driver).is_empty());
    assert!(view.read_with(cx, |view, cx| view.terminal.read(cx).text_error().is_some()));
}

#[gpui_kit::test]
fn same_state_renegotiation_discards_ownership_but_ordinary_output_preserves_it(
    cx: &mut TestAppContext,
) {
    let (handle, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(cx, &transport, 0, Event::Output(b"\x1b[>3u".to_vec()));
    drain(&driver);
    for output in [
        b"\x1b[=1u\x1b[=3u".as_slice(),
        b"\x1b[>3u\x1b[<u",
        b"\x1b[?1049h\x1b[?1049l",
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            down(window, "up", false, false, cx);
        })
        .unwrap();
        assert_eq!(drain(&driver), [b"\x1b[A".to_vec()]);
        emit(cx, &transport, 0, Event::Output(output.to_vec()));
        cx.update_window(handle, |_, window, cx| up(window, "up", cx))
            .unwrap();
        assert!(
            drain(&driver).is_empty(),
            "new negotiation invalidates held keys"
        );
    }
    cx.update_window(handle, |_, window, cx| down(window, "up", false, false, cx))
        .unwrap();
    drain(&driver);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"ordinary output\x1b[?u".to_vec()),
    );
    assert_eq!(drain(&driver), [b"\x1b[?3u".to_vec()]);
    cx.update_window(handle, |_, window, cx| up(window, "up", cx))
        .unwrap();
    assert_eq!(drain(&driver), [b"\x1b[1;1:3A".to_vec()]);
}

#[gpui_kit::test]
fn kitty_shifted_identity_does_not_use_platform_control_text(cx: &mut TestAppContext) {
    let (handle, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    emit(cx, &transport, 0, Event::Output(b"\x1b[>7u".to_vec()));
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let mut keystroke = Keystroke::parse("ctrl-shift-a").unwrap();
        keystroke.key_char = Some("\x01".into());
        window.dispatch_event(
            KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        up(window, "ctrl-shift-a", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        [b"\x1b[97;6u".to_vec(), b"\x1b[97;6:3u".to_vec()]
    );
}
