//! Renames keep their original host and folder while the browser moves on.
use super::*;

#[gpui_kit::test]
fn rename_dialog_pins_old_host_and_directory_without_refreshing_new_browser(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let original = Arc::new(OperationsFs::writable());
    let replacement = Arc::new(OperationsFs::writable());
    let (handle, _, item, panel) = visible_panel(cx, original.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, RENAME);
    item.update(cx, |item, cx| {
        item.session.target.host = "replacement".into();
        item.session.fs = Some(replacement.clone());
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/elsewhere".into()), cx)
    });
    cx.run_until_parked();
    let calls = replacement.calls();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert!(
        original
            .calls()
            .contains(&"rename:/home/test/a.txt->/home/test/renamed.txt".into())
    );
    assert_eq!(
        replacement.calls(),
        calls,
        "stale modal refresh must not issue a listing on the new host or directory"
    );
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/elsewhere")
        )
    });
}

#[gpui_kit::test]
fn navigation_on_same_filesystem_keeps_rename_target_and_ignores_stale_refresh(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let fs = Arc::new(OperationsFs::writable());
    let (handle, _, _, panel) = visible_panel(cx, fs.clone(), directory.path());
    context_menu(cx, handle, false, 1);
    choose(cx, handle, RENAME);
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/elsewhere".into()), cx)
    });
    cx.run_until_parked();
    let listings = fs
        .calls()
        .iter()
        .filter(|call| call.starts_with("list:"))
        .count();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert!(
        fs.calls()
            .contains(&"rename:/home/test/a.txt->/home/test/renamed.txt".into())
    );
    assert_eq!(
        fs.calls()
            .iter()
            .filter(|call| call.starts_with("list:"))
            .count(),
        listings
    );
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/elsewhere")
        )
    });
}

#[gpui_kit::test]
fn local_navigation_during_rename_keeps_original_path_without_refreshing_new_folder(
    cx: &mut TestAppContext,
) {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("a.txt"), b"source").unwrap();
    std::fs::write(destination.path().join("a.txt"), b"destination").unwrap();
    let (deps, local) = fixture_deps_at(source.path());
    seed_fixture_tree(&local, destination.path());
    let (handle, workspace, _, panel) =
        panel_fixture_with(cx, Arc::new(OperationsFs::writable()), deps);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        cx.set_reduce_motion(true);
        workspace.update(cx, |workspace, cx| {
            add_explorer(workspace, panel.clone(), window, cx)
        });
        panel.update(cx, |panel, cx| {
            panel.navigate_local(source.path().into(), cx)
        });
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1100.)));
    cx.run_until_parked();
    context_menu(cx, handle, true, 0);
    choose(cx, handle, RENAME);
    panel.update(cx, |panel, cx| {
        panel.navigate_local(destination.path().into(), cx)
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert_eq!(
        native_bytes(local.as_ref(), &source.path().join("renamed.txt")),
        b"source"
    );
    assert_eq!(
        native_bytes(local.as_ref(), &destination.path().join("a.txt")),
        b"destination"
    );
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).path().as_deref(),
            Some(destination.path())
        );
        assert_eq!(panel.local_listing(cx).names(), ["a.txt"]);
    });
}
