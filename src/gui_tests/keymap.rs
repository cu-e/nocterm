//! The root captures one keymap across page factories and actual NewWindow actions.
use super::*;
use nocterm_keymap::KeymapHandle;
fn open_keymap(
    handle: AnyWindowHandle,
    workspace: Option<&Entity<Workspace>>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        if let Some(workspace) = workspace {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
        }
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::OpenKeymap), cx);
    })
    .unwrap();
    cx.run_until_parked();
    filter_new_tab(handle, cx);
}
fn filter_new_tab(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("keymap-search", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input("workspace::newtab", cx);
        window.render_frame(cx);
        assert!(window.try_find(("keymap-row", 1usize)).is_none());
    })
    .unwrap();
    cx.run_until_parked();
}
fn rebind_new_tab(handle: AnyWindowHandle, keys: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.click(("keymap-keys", 0usize), cx);
        window.press(keys, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn assert_shortcut_opens_menu(handle: AnyWindowHandle, keys: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.press(keys, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("menu-new-connection").visible());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn root_new_window_keeps_shared_keymap_after_source_window_closes(cx: &mut TestAppContext) {
    let (booted, first, first_workspace, second, second_workspace) = cx.update(|cx| {
        let booted = boot(
            tempfile::tempdir().unwrap(),
            Default::default(),
            Arc::new(MockTransport::default()),
            false,
            cx,
        );
        let (first, a) = open_workspace(&booted, cx);
        let (second, b) = open_workspace(&booted, cx);
        (booted, first, a, second, b)
    });
    open_keymap(first, Some(&first_workspace), cx);
    open_keymap(second, Some(&second_workspace), cx);
    rebind_new_tab(first, "alt-t", cx);
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
    })
    .unwrap();
    let weak = first_workspace.downgrade();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_workspace);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    cx.update_window(second, |_, window, cx| {
        window.click(("keymap-reset", 0usize), cx);
        window.render_frame(cx);
        assert!(window.try_find(("keymap-reset", 0usize)).is_none());
    })
    .unwrap();
    rebind_new_tab(second, "alt-y", cx);
    let before = cx.update(|cx| cx.windows());
    cx.update_window(second, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::NewWindow), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let third = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.iter().any(|old| old.window_id() == w.window_id()))
            .unwrap()
    });
    open_keymap(third, None, cx);
    cx.update_window(third, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
    })
    .unwrap();
    assert_shortcut_opens_menu(second, "alt-y", cx);
    assert_shortcut_opens_menu(third, "alt-y", cx);
    cx.update(|cx| {
        assert!(
            booted
                .keymap()
                .bindings(cx)
                .iter()
                .any(|b| b.action == "workspace::NewTab" && b.keystrokes == "alt-y")
        )
    });
}

fn native_pages(
    file: &std::path::Path,
    cx: &mut TestAppContext,
) -> (KeymapHandle, AnyWindowHandle, AnyWindowHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        let keymap = crate::keymap::load(Some(file.to_owned()), cx);
        let mut page = || {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| nocterm_keymap_ui::KeymapView::new(keymap.clone(), window, cx))
            })
            .unwrap()
            .0
        };
        let a = page();
        let b = page();
        (keymap, a, b)
    })
}
#[gpui_kit::test]
fn native_keymap_save_failure_refreshes_both_pages_then_recovers(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("keymap.toml");
    let (keymap, first, second) = native_pages(&file, cx);
    filter_new_tab(first, cx);
    filter_new_tab(second, cx);
    std::fs::create_dir(&file).unwrap();
    rebind_new_tab(first, "alt-t", cx);
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("keymap-file-error").is_some());
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
        window.click("keymap-search-keys", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(keymap.error(cx).is_some()));
    std::fs::remove_dir(&file).unwrap();
    rebind_new_tab(first, "alt-y", cx);
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("keymap-file-error").is_none());
        assert!(window.try_find(("keymap-reset", 0usize)).is_some());
    })
    .unwrap();
    cx.update(|cx| assert!(keymap.error(cx).is_none()));
    assert!(file.is_file());
}
