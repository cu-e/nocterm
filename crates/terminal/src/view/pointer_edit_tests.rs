//! Editing the screen must end an edge drag, including its recurring timer.
use super::*;

fn edit_during_autoscroll(
    cx: &mut TestAppContext,
    edit: impl FnOnce(&mut Window, &mut App),
    expected: &[u8],
) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    tick(cx);
    assert!(offset(&view, cx) > 0);
    assert!(selection(&view, cx).is_some());
    cx.update_window(handle, |_, window, cx| edit(window, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(drain(&driver), [expected.to_vec()]);
    assert_eq!(
        offset(&view, cx),
        0,
        "accepted input must return to the live screen"
    );
    assert_eq!(selection(&view, cx), None);
    tick(cx);
    assert_eq!(
        offset(&view, cx),
        0,
        "old selection timer scrolled after accepted input"
    );
    assert_eq!(selection(&view, cx), None);
    assert!(
        drain(&driver).is_empty(),
        "selection timer must not generate session input"
    );
}

#[gpui_kit::test]
fn accepted_ctrl_c_cancels_edge_autoscroll(cx: &mut TestAppContext) {
    edit_during_autoscroll(cx, |window, cx| window.press("ctrl-c", cx), &[3]);
}

#[gpui_kit::test]
fn accepted_plain_key_cancels_edge_autoscroll(cx: &mut TestAppContext) {
    edit_during_autoscroll(cx, |window, cx| window.press("a", cx), b"a");
}

#[gpui_kit::test]
fn accepted_text_commit_cancels_edge_autoscroll(cx: &mut TestAppContext) {
    edit_during_autoscroll(cx, |window, cx| window.input("界", cx), "界".as_bytes());
}

#[gpui_kit::test]
fn accepted_clipboard_paste_cancels_edge_autoscroll(cx: &mut TestAppContext) {
    edit_during_autoscroll(
        cx,
        |window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
            window.dispatch_action(Box::new(Paste), cx);
        },
        b"pasted",
    );
}

#[gpui_kit::test]
fn clear_selection_cancels_edge_autoscroll_without_changing_viewport(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    history(cx, &transport);
    let g = geometry(handle, &view, cx);
    down(handle, at(g, 2.2, 5.5), MouseButton::Left, false, 1, cx);
    motion(handle, at(g, 2.2, -1.0), Some(MouseButton::Left), false, cx);
    tick(cx);
    let before = offset(&view, cx);
    assert!(before > 0);
    assert!(selection(&view, cx).is_some());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.execute(ItemCommand::ClearSelection, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selection(&view, cx), None);
    assert_eq!(offset(&view, cx), before);
    tick(cx);
    assert_eq!(
        offset(&view, cx),
        before,
        "old selection timer scrolled after ClearSelection"
    );
    assert_eq!(selection(&view, cx), None);
    assert!(drain(&scripted_driver(&transport, 0)).is_empty());
}
