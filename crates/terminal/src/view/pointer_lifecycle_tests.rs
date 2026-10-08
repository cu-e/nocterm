use super::*;

#[gpui_kit::test]
fn focus_loss_clears_program_drag_and_reentry_reports_current_cell(cx: &mut TestAppContext) {
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
    cx.update_window(handle, |_, window, cx| window.blur(cx))
        .unwrap();
    cx.run_until_parked();
    motion(handle, at(g, 3.2, 2.5), Some(MouseButton::Left), false, cx);
    up(handle, at(g, 3.2, 2.5), MouseButton::Left, cx);
    assert!(
        drain(&driver).is_empty(),
        "lost-focus press ownership leaked into later events"
    );
    down(handle, at(g, 3.2, 2.5), MouseButton::Left, false, 1, cx);
    up(handle, at(g, 3.2, 2.5), MouseButton::Left, cx);
    assert_eq!(
        drain(&driver),
        [b"\x1b[<0;4;3M".to_vec(), b"\x1b[<0;4;3m".to_vec()]
    );
}

#[gpui_kit::test]
fn mouse_tracking_disable_and_reenable_drop_stale_button_ownership(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, false, 1, cx);
    drain(&driver);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002l\x1b[?1002h".to_vec()),
    );
    motion(handle, at(g, 3.2, 2.5), Some(MouseButton::Left), false, cx);
    up(handle, at(g, 3.2, 2.5), MouseButton::Left, cx);
    assert!(
        drain(&driver).is_empty(),
        "mode roundtrip reused a press from old tracking state"
    );
}

#[gpui_kit::test]
fn hover_mode_reenable_reports_the_same_cell_once(cx: &mut TestAppContext) {
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
    motion(handle, at(g, 2.2, 2.5), None, false, cx);
    assert_eq!(drain(&driver), [b"\x1b[<35;3;3M".to_vec()]);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1003l\x1b[?1003h".to_vec()),
    );
    motion(handle, at(g, 2.2, 2.5), None, false, cx);
    motion(handle, at(g, 2.4, 2.5), None, false, cx);
    assert_eq!(
        drain(&driver),
        [b"\x1b[<35;3;3M".to_vec()],
        "new tracking session must not inherit old cell deduplication"
    );
}

#[gpui_kit::test]
fn disconnect_reconnect_and_screen_switch_cancel_old_selection_drag(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    emit(cx, &transport, 0, Event::Closed(CloseReason::ClosedByUser));
    let before_offset = offset(&view, cx);
    let before_selection = selection(&view, cx);
    tick(cx);
    motion(handle, at(g, 6.2, 3.5), Some(MouseButton::Left), false, cx);
    assert_eq!(offset(&view, cx), before_offset);
    assert_eq!(
        selection(&view, cx),
        before_selection,
        "closed session retained active selection drag"
    );
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(Reconnect), cx)
    })
    .unwrap();
    cx.run_until_parked();
    emit(cx, &transport, 1, Event::Connected);
    let before_selection = selection(&view, cx);
    motion(handle, at(g, 10.2, 8.5), Some(MouseButton::Left), false, cx);
    assert_eq!(
        selection(&view, cx),
        before_selection,
        "reconnected session revived previous drag"
    );
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    emit(cx, &transport, 1, Event::Output(b"\x1b[?1049h".to_vec()));
    let screen_selection = selection(&view, cx);
    motion(handle, at(g, 8.2, 3.5), Some(MouseButton::Left), false, cx);
    tick(cx);
    assert_eq!(offset(&view, cx), 0);
    assert_eq!(
        selection(&view, cx),
        screen_selection,
        "drag from main screen mutated alternate screen"
    );
}

#[gpui_kit::test]
fn changed_held_button_cancels_program_drag_instead_of_reporting_wrong_button(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, false, 1, cx);
    drain(&driver);
    motion(handle, at(g, 3.2, 2.5), Some(MouseButton::Right), false, cx);
    up(handle, at(g, 3.2, 2.5), MouseButton::Left, cx);
    assert!(
        drain(&driver).is_empty(),
        "right-button event inherited left-button press ownership"
    );
}

#[gpui_kit::test]
fn missing_held_button_clears_program_drag_ownership(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, false, 1, cx);
    drain(&driver);
    motion(handle, at(g, 3.2, 2.5), None, false, cx);
    motion(handle, at(g, 4.2, 2.5), Some(MouseButton::Left), false, cx);
    up(handle, at(g, 4.2, 2.5), MouseButton::Left, cx);
    assert!(
        drain(&driver).is_empty(),
        "missing-button event did not cancel old ownership"
    );
}

#[gpui_kit::test]
fn ordinary_output_preserves_program_drag_ownership(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"\x1b[?1002h\x1b[?1006h".to_vec()),
    );
    let driver = scripted_driver(&transport, 0);
    let g = geometry(handle, &view, cx);
    drain(&driver);
    down(handle, at(g, 2.2, 2.5), MouseButton::Left, false, 1, cx);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(ScrollPageUp), cx);
    })
    .unwrap();
    cx.run_until_parked();
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"ordinary application output".to_vec()),
    );
    motion(handle, at(g, 3.2, 2.5), Some(MouseButton::Left), false, cx);
    up(handle, at(g, 3.2, 2.5), MouseButton::Left, cx);
    assert_eq!(
        drain(&driver),
        [b"\x1b[<32;4;3M".to_vec(), b"\x1b[<0;4;3m".to_vec()]
    );
}

#[gpui_kit::test]
fn ordinary_output_keeps_stationary_edge_autoscroll_active(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    tick(cx);
    assert!(offset(&view, cx) > 0);
    emit(
        cx,
        &transport,
        0,
        Event::Output(b"ongoing PTY output".to_vec()),
    );
    let before = offset(&view, cx);
    let before_selection = selection(&view, cx);
    tick(cx);
    assert!(
        offset(&view, cx) > before,
        "ordinary output cancelled autoscroll"
    );
    assert_ne!(selection(&view, cx), before_selection);
    up(handle, at(g, -10.0, -2.0), MouseButton::Left, cx);
}

#[gpui_kit::test]
fn closed_terminal_still_allows_a_new_read_only_selection(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"alpha beta".to_vec()));
    emit(cx, &transport, 0, Event::Closed(CloseReason::ClosedByUser));
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 0.5), MouseButton::Left, false, 2, cx);
    up(handle, at(g, 2.2, 0.5), MouseButton::Left, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("alpha"));
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(native_input::Copy), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("alpha")
        )
    });
    assert!(drain(&scripted_driver(&transport, 0)).is_empty());
}

#[gpui_kit::test]
fn activating_another_window_cancels_autoscroll_despite_retained_terminal_focus(
    cx: &mut TestAppContext,
) {
    struct OtherWindow;
    impl Render for OtherWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    tick(cx);
    assert!(
        offset(&view, cx) > 0,
        "fixture must first demonstrate active autoscroll"
    );
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            window.activate_window();
            cx.new(|_| OtherWindow)
        })
        .unwrap();
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).focus_handle.is_focused(window));
        assert_ne!(cx.active_window(), Some(window.window_handle()));
    })
    .unwrap();
    let before = offset(&view, cx);
    let before_selection = selection(&view, cx);
    tick(cx);
    assert_eq!(
        offset(&view, cx),
        before,
        "inactive window continued selection autoscroll"
    );
    assert_eq!(selection(&view, cx), before_selection);
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    motion(handle, at(g, 8.2, 3.5), Some(MouseButton::Left), false, cx);
    tick(cx);
    assert_eq!(
        offset(&view, cx),
        before,
        "reactivation revived previous drag"
    );
    assert_eq!(selection(&view, cx), before_selection);
}
