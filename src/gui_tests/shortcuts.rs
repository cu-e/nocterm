//! Shortcuts pressed in a connected terminal reach the workspace.
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, test::TestWindowExt as _};

use super::fixture;

#[gpui_kit::test]
fn new_tab_shortcut_from_connected_terminal_opens_menu(cx: &mut TestAppContext) {
    let (handle, _, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-t"
            } else {
                "ctrl-shift-t"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("menu-new-connection").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_shortcut_from_connected_terminal_opens_settings(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-,"
            } else {
                "ctrl-,"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).items().count(), 2);
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn command_palette_takes_typing_and_runs_the_command_from_the_terminal(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-shift-p"
            } else {
                "ctrl-shift-p"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dialog").is_some())
    })
    .unwrap();
    // Typing goes straight into the palette's search field.
    cx.simulate_input(handle, "open settings");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .find_item::<nocterm_settings_ui::SettingsView>()
                .is_some(),
            "the command ran"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn close_shortcut_from_connected_terminal_closes_tab(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-shift-w"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.items().count()),
        0
    );
}
