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
    panel.update(cx, |panel, cx| panel.load(Some("/elsewhere".into()), cx));
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
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.path.as_deref(), Some("/elsewhere"))
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
    panel.update(cx, |panel, cx| panel.load(Some("/elsewhere".into()), cx));
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
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.path.as_deref(), Some("/elsewhere"))
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
    let (handle, _, _, panel) =
        visible_panel(cx, Arc::new(OperationsFs::writable()), source.path());
    context_menu(cx, handle, true, 0);
    choose(cx, handle, RENAME);
    panel.update(cx, |panel, cx| {
        panel.load_local(destination.path().into(), cx)
    });
    cx.run_until_parked();
    let generation = panel.read_with(cx, |panel, _| panel.local.generation);
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("renamed.txt", cx);
    })
    .unwrap();
    button(cx, handle, "file-dialog-submit");
    assert_eq!(
        std::fs::read(source.path().join("renamed.txt")).unwrap(),
        b"source"
    );
    assert_eq!(
        std::fs::read(destination.path().join("a.txt")).unwrap(),
        b"destination"
    );
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path.as_deref(), Some(destination.path()));
        assert_eq!(panel.local.generation, generation);
    });
}
