//! Pointer input is dispatched through the painted terminal element, including outside drags.
use super::*;
use gpui_kit::{InputEvent as _, ScrollDelta, point, px};

fn geometry(
    handle: AnyWindowHandle,
    view: &Entity<TerminalView>,
    cx: &mut TestAppContext,
) -> Geometry {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.read(cx).geometry.get().unwrap()
    })
    .unwrap()
}
fn at(g: Geometry, col: f32, row: f32) -> Point<Pixels> {
    point(
        g.origin.x + g.cell_width * col,
        g.origin.y + g.line_height * row,
    )
}
fn motion(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    held: Option<MouseButton>,
    shift: bool,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: held,
                modifiers: gpui_kit::Modifiers {
                    shift,
                    ..Default::default()
                },
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
fn down(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    button: MouseButton,
    shift: bool,
    clicks: usize,
    cx: &mut TestAppContext,
) {
    motion(handle, position, None, shift, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_event(
            MouseDownEvent {
                position,
                button,
                click_count: clicks,
                modifiers: gpui_kit::Modifiers {
                    shift,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
fn up(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    button: MouseButton,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            MouseUpEvent {
                position,
                button,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
fn wheel(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    delta: ScrollDelta,
    shift: bool,
    cx: &mut TestAppContext,
) {
    motion(handle, position, None, shift, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta,
                modifiers: gpui_kit::Modifiers {
                    shift,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}
fn tick(cx: &mut TestAppContext) {
    // Advance in steps: a recurring timer must run while the pointer is stationary.
    for _ in 0..8 {
        cx.executor().advance_clock(Duration::from_millis(25));
        cx.run_until_parked();
    }
}
fn offset(view: &Entity<TerminalView>, cx: &mut TestAppContext) -> usize {
    view.read_with(cx, |v, cx| v.terminal.read(cx).emulator().display_offset())
}
fn selection(view: &Entity<TerminalView>, cx: &mut TestAppContext) -> Option<String> {
    view.read_with(cx, |v, cx| v.terminal.read(cx).emulator().selection_text())
}
fn history(cx: &mut TestAppContext, transport: &Scripted) {
    let output = (0..200)
        .map(|i| format!("history-{i:03}\r\n"))
        .collect::<String>();
    emit(cx, transport, 0, Event::Output(output.into_bytes()));
}

#[gpui_kit::test]
fn stationary_edge_drag_scrolls_history_reverses_and_release_outside_stops(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    down(
        handle,
        at(g, 5.2, f32::from(g.rows) - 2.5),
        MouseButton::Left,
        false,
        1,
        cx,
    );
    motion(handle, at(g, 2.2, -2.0), Some(MouseButton::Left), false, cx);
    let initial = offset(&view, cx);
    let text = selection(&view, cx).unwrap();
    tick(cx);
    let scrolled = offset(&view, cx);
    assert!(
        scrolled > initial,
        "stationary drag above grid did not scroll history"
    );
    let expanded = selection(&view, cx).unwrap();
    assert!(
        expanded.len() > text.len(),
        "selection did not extend into newly exposed history"
    );
    motion(
        handle,
        at(g, 2.2, f32::from(g.rows) + 2.0),
        Some(MouseButton::Left),
        false,
        cx,
    );
    tick(cx);
    assert!(
        offset(&view, cx) < scrolled,
        "lower edge must reverse scrolling"
    );
    up(handle, at(g, -10.0, -2.0), MouseButton::Left, cx);
    let ended = offset(&view, cx);
    let final_text = selection(&view, cx);
    tick(cx);
    assert_eq!(offset(&view, cx), ended);
    assert_eq!(selection(&view, cx), final_text);
}

#[gpui_kit::test]
fn missing_held_button_and_focus_loss_cancel_selection_drag(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    for lose_focus in [false, true] {
        down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
        motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
        if lose_focus {
            cx.update_window(handle, |_, window, cx| {
                window.blur(cx);
            })
            .unwrap();
            cx.run_until_parked();
        } else {
            motion(handle, at(g, 3.2, 2.5), None, false, cx);
        }
        let before = selection(&view, cx);
        let before_offset = offset(&view, cx);
        motion(handle, at(g, 10.2, 8.5), Some(MouseButton::Left), false, cx);
        tick(cx);
        assert_eq!(
            selection(&view, cx),
            before,
            "cancelled drag resumed, focus_loss={lose_focus}"
        );
        assert_eq!(offset(&view, cx), before_offset);
    }
}

#[gpui_kit::test]
fn double_click_words_triple_click_lines_and_copy_on_release(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"alpha beta\r\nsecond line".to_vec()),
    );
    cx.update(|cx| nocterm_ui::edit_settings(cx, |s| s.terminal.copy_on_select = true).detach());
    cx.run_until_parked();
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 0.5), MouseButton::Left, false, 2, cx);
    up(handle, at(g, 2.2, 0.5), MouseButton::Left, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("alpha"));
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("alpha")
        )
    });
    down(handle, at(g, 2.2, 1.5), MouseButton::Left, false, 3, cx);
    up(handle, at(g, 2.2, 1.5), MouseButton::Left, cx);
    assert_eq!(selection(&view, cx).unwrap().trim_end(), "second line");
    assert!(drain(&scripted_driver(&transport, 0)).is_empty());
}

#[gpui_kit::test]
fn sgr_drag_reports_exact_cells_and_unrelated_release_keeps_owner(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, false, 1, cx);
    assert_eq!(drain(&driver), [b"\x1b[<0;3;3M".to_vec()]);
    motion(handle, at(g, 3.2, 2.5), Some(MouseButton::Left), false, cx);
    motion(handle, at(g, 3.4, 2.5), Some(MouseButton::Left), false, cx);
    assert_eq!(drain(&driver), [b"\x1b[<32;4;3M".to_vec()]);
    up(handle, at(g, 3.2, 2.5), MouseButton::Right, cx);
    assert!(
        drain(&driver).is_empty(),
        "unrelated release ended left-button ownership"
    );
    motion(handle, at(g, 4.2, 2.5), Some(MouseButton::Left), false, cx);
    up(handle, at(g, 4.2, 2.5), MouseButton::Left, cx);
    assert_eq!(
        drain(&driver),
        [b"\x1b[<32;5;3M".to_vec(), b"\x1b[<0;5;3m".to_vec()]
    );
    assert_eq!(selection(&view, cx), None);
}

#[gpui_kit::test]
fn shift_selection_and_wheel_bypass_program_mouse_tracking(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1003h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, true, 1, cx);
    motion(handle, at(g, 6.8, 3.5), Some(MouseButton::Left), true, cx);
    up(handle, at(g, 6.8, 3.5), MouseButton::Left, cx);
    assert!(selection(&view, cx).is_some());
    assert!(
        drain(&driver).is_empty(),
        "Shift selection leaked program mouse reports"
    );
    let before = offset(&view, cx);
    wheel(
        handle,
        at(g, 2.2, 2.5),
        ScrollDelta::Lines(point(0.0, 3.0)),
        true,
        cx,
    );
    assert!(
        offset(&view, cx) > before,
        "Shift wheel did not scroll local history"
    );
    assert!(drain(&driver).is_empty());
}

#[gpui_kit::test]
fn fractional_wheel_accumulates_and_program_reports_are_bounded(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    let position = at(g, 2.2, 2.5);
    wheel(
        handle,
        position,
        ScrollDelta::Pixels(point(px(0.0), g.line_height * 0.4)),
        false,
        cx,
    );
    assert_eq!(offset(&view, cx), 0);
    wheel(
        handle,
        position,
        ScrollDelta::Pixels(point(px(0.0), g.line_height * 0.7)),
        false,
        cx,
    );
    assert_eq!(offset(&view, cx), 1);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1000h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    wheel(
        handle,
        position,
        ScrollDelta::Lines(point(0.0, 1000.0)),
        false,
        cx,
    );
    let reports = drain(&driver);
    assert_eq!(reports.len(), 10);
    assert!(reports.iter().all(|r| r == b"\x1b[<64;3;3M"));
}

#[gpui_kit::test]
fn alternate_screen_wheel_arrows_are_bounded_for_a_large_fling(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1049h\x1b[?1007h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    let g = geometry(handle, &view, cx);
    wheel(
        handle,
        at(g, 2.2, 2.5),
        ScrollDelta::Lines(point(0.0, -1000.0)),
        false,
        cx,
    );
    let reports = drain(&driver);
    assert_eq!(
        reports.len(),
        10,
        "alternate-screen fling must not flood the session"
    );
    assert!(reports.iter().all(|r| r == b"\x1b[B"));
}

#[gpui_kit::test]
fn extreme_signed_wheel_deltas_do_not_panic_or_flood_protocol_input(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    for mode in [
        b"\x1b[?1000h\x1b[?1006h".as_slice(),
        b"\x1b[?1000l\x1b[?1049h\x1b[?1007h",
    ] {
        emit(cx, &transport, 0, Event::Output(mode.to_vec()));
        let g = geometry(handle, &view, cx);
        drain(&driver);
        // Exceeds i32's range while remaining finite after conversion to pixels.
        for delta in [-1.0e10, 1.0e10] {
            wheel(
                handle,
                at(g, 2.2, 2.5),
                ScrollDelta::Lines(point(0.0, delta)),
                false,
                cx,
            );
            let reports = drain(&driver);
            assert!(
                !reports.is_empty(),
                "finite nonzero wheel delta must scroll"
            );
            assert!(
                reports.len() <= 10,
                "fling generated {} reports",
                reports.len()
            );
        }
    }
}

#[path = "pointer_lifecycle_tests.rs"]
mod lifecycle;

#[path = "pointer_edit_tests.rs"]
mod editing;

#[gpui_kit::test]
fn shift_wheel_scrolls_history_without_program_reports(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1000h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    let g = geometry(handle, &view, cx);
    wheel(
        handle,
        at(g, 2.2, 2.5),
        ScrollDelta::Lines(point(0.0, 3.0)),
        true,
        cx,
    );
    assert_eq!(offset(&view, cx), 3);
    assert!(
        drain(&driver).is_empty(),
        "Shift wheel leaked program mouse reports"
    );
}

#[gpui_kit::test]
fn legacy_mouse_clicks_and_sgr_modifiers_follow_element_cell_mapping(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    emit(cx, &transport, 0, Event::Output(b"\x1b[?1000h".to_vec()));
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Right, false, 1, cx);
    up(handle, at(g, 2.2, 2.5), MouseButton::Right, cx);
    assert_eq!(
        drain(&driver),
        [
            b"\x1b[M\x22\x23\x23".to_vec(),
            b"\x1b[M\x23\x23\x23".to_vec()
        ]
    );
    emit(cx, &transport, 0, Event::Output(b"\x1b[?1006h".to_vec()));
    motion(handle, at(g, 2.2, 2.5), None, false, cx);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: at(g, 2.2, 2.5),
                click_count: 1,
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    alt: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    assert_eq!(drain(&driver), [b"\x1b[<24;3;3M".to_vec()]);
}

#[gpui_kit::test]
fn hovering_outside_the_grid_and_horizontal_wheel_do_not_report(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1003h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    motion(handle, at(g, -10.0, -2.0), None, false, cx);
    assert!(drain(&driver).is_empty());
    motion(handle, at(g, 2.2, 2.5), None, false, cx);
    assert_eq!(drain(&driver), [b"\x1b[<35;3;3M".to_vec()]);
    wheel(
        handle,
        at(g, 2.2, 2.5),
        ScrollDelta::Lines(point(10.0, 0.0)),
        false,
        cx,
    );
    assert!(drain(&driver).is_empty());
}
