//! Both panes expose the same selection gestures while keeping separate paths.
use super::*;
use gpui_kit::{Modifiers, test::TestWindowExt as _};

#[gpui_kit::test]
fn both_panes_preserve_context_selection_and_reset_the_range_anchor_on_navigation(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(directory.path().join(name), b"local").unwrap();
    }
    let (handle, workspace, _, panel) =
        panel_fixture_at(cx, transfer_remote(false), directory.path());
    transfer_ui::show_local(cx, handle, &workspace, &panel, directory.path());
    for local in [true, false] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let group = if local { "local-entry" } else { "remote-entry" };
            let first = window.find((group, 0usize)).bounds().center();
            let third = window.find((group, 2usize)).bounds().center();
            pointer_click(window, first, Default::default(), cx);
            pointer_click(
                window,
                third,
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
                cx,
            );
            let middle = window.find((group, 1usize)).bounds().center();
            pointer_click(
                window,
                middle,
                Modifiers {
                    platform: true,
                    ..Default::default()
                },
                cx,
            );
        })
        .unwrap();
        assert_selection(&panel, local, &[0, 2], cx);
        operations_ui::context_menu(cx, handle, local, 2);
        assert_selection(&panel, local, &[0, 2], cx);
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        operations_ui::context_menu(cx, handle, local, 1);
        assert_selection(&panel, local, &[1], cx);
        cx.update_window(handle, |_, window, cx| window.press("escape", cx))
            .unwrap();
        panel.update(cx, |panel, cx| {
            if local {
                panel.navigate_local(directory.path().to_owned(), cx);
            } else {
                panel.navigate_remote(Some("/home/test".into()), cx);
            }
        });
        cx.run_until_parked();
        assert_selection(&panel, local, &[], cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let group = if local { "local-entry" } else { "remote-entry" };
            let third = window.find((group, 2usize)).bounds().center();
            pointer_click(
                window,
                third,
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
                cx,
            );
        })
        .unwrap();
        assert_selection(&panel, local, &[2], cx);
    }
}
fn assert_selection(
    panel: &Entity<FilesPanel>,
    local: bool,
    expected: &[usize],
    cx: &mut TestAppContext,
) {
    panel.read_with(cx, |panel, cx| {
        if local {
            assert_eq!(panel.local_listing(cx).selected(), expected);
        } else {
            assert_eq!(panel.remote_listing(cx).selected(), expected);
        }
    });
}
