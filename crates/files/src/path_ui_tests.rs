//! The location editor's actual FilesPanel navigation subscriptions.
use super::super::*;
use gpui_kit::test::TestWindowExt as _;

struct PanelHarness(Entity<FilesPanel>);
impl Render for PanelHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn show_panel(
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    panel: &Entity<FilesPanel>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
}

fn enter_path(handle: AnyWindowHandle, pane: &'static str, draft: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within(pane).click("explorer-path-display", cx);
        window.render_frame(cx);
        window.input(draft, cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn clicking_local_location_and_enter_reloads_panel_then_file_error_keeps_draft(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("資料 folder");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("file.txt"), b"content").unwrap();
    let fs = Arc::new(PendingFs::default());
    let (handle, workspace, _, panel) = panel_fixture(cx, fs.clone());
    show_panel(handle, &workspace, &panel, cx);
    panel.update(cx, |panel, cx| {
        panel.load_local(directory.path().into(), cx)
    });
    cx.run_until_parked();
    fs.take_request("/home/test").send(Ok(vec![])).unwrap();
    cx.run_until_parked();

    enter_path(handle, "local-browser", "資料 folder", cx);
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path.as_deref(), Some(child.as_path()));
        assert_eq!(panel.local.entries[0].name, "file.txt");
    });
    enter_path(handle, "local-browser", "file.txt", cx);
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path.as_deref(), Some(child.as_path()));
        assert!(panel.local.error.is_some());
    });
    // A second Enter submits the unchanged failed draft through the real field.
    // Replace the file with a folder so this retry can succeed.
    std::fs::remove_file(child.join("file.txt")).unwrap();
    std::fs::create_dir(child.join("file.txt")).unwrap();
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path, Some(child.join("file.txt")));
        assert!(panel.local.error.is_none());
    });
}

#[gpui_kit::test]
fn clicking_remote_location_and_enter_uses_remote_fs_and_keeps_failed_file_draft(
    cx: &mut TestAppContext,
) {
    let fs = Arc::new(PendingFs::default());
    let (handle, workspace, _, panel) = panel_fixture(cx, fs.clone());
    show_panel(handle, &workspace, &panel, cx);
    cx.run_until_parked();
    fs.take_request("/home/test").send(Ok(vec![])).unwrap();
    cx.run_until_parked();
    enter_path(handle, "remote-browser", "~/資料 folder", cx);
    fs.take_request("/home/test/資料 folder")
        .send(Ok(vec![entry("file.txt", EntryKind::File)]))
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.browser.path.as_deref(),
            Some("/home/test/資料 folder")
        );
        assert_eq!(panel.browser.entries[0].name, "file.txt");
    });
    // Non-home folders also receive an independent statistics listing.
    fs.take_request("/home/test/資料 folder")
        .send(Ok(vec![entry("file.txt", EntryKind::File)]))
        .unwrap();
    cx.run_until_parked();
    enter_path(handle, "remote-browser", "file.txt", cx);
    fs.take_request("/home/test/資料 folder/file.txt")
        .send(Err(FsError::PermissionDenied {
            path: "/home/test/資料 folder/file.txt".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.browser.path.as_deref(),
            Some("/home/test/資料 folder")
        );
        assert!(panel.browser.error.is_some());
    });
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    fs.take_request("/home/test/資料 folder/file.txt")
        .send(Ok(vec![]))
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.browser.path.as_deref(),
            Some("/home/test/資料 folder/file.txt")
        );
        assert!(panel.browser.error.is_none());
    });
}

#[gpui_kit::test]
fn minimum_remote_and_local_panes_keep_tab_selected_match_clickable(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    let directory = tempfile::tempdir().unwrap();
    for index in 0..20 {
        std::fs::write(
            directory.path().join(format!("file-{index:02}")),
            b"content",
        )
        .unwrap();
    }
    let fs = Arc::new(PendingFs::default());
    let (handle, _, _, panel) = panel_fixture(cx, fs.clone());
    cx.update_window(handle, |_, window, cx| {
        window.replace_root(cx, |_, _| PanelHarness(panel.clone()));
        panel.update(cx, |panel, cx| {
            panel.load_local(directory.path().into(), cx)
        });
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(320.), px(400.)));
    cx.run_until_parked();
    fs.take_request("/home/test").send(Ok(vec![])).unwrap();
    cx.run_until_parked();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (pane, index, minimum, draft) in [
            ("remote-browser", 0, px(120.), "/home/test/"),
            ("local-browser", 1, px(140.), ""),
        ] {
            if index == 0 {
                panel.update(cx, |panel, cx| panel.load(Some("/home/test".into()), cx));
                cx.run_until_parked();
                fs.take_request("/home/test").send(Ok(vec![])).unwrap();
                cx.run_until_parked();
            }
            cx.update_window(handle, |_, window, cx| {
                Theme::change(mode, None, cx);
                window.render_frame(cx);
                let divider = panel.read(cx).divider.clone();
                divider.update(cx, |divider, cx| {
                    divider.resize_panel(index, minimum, window, cx)
                });
                window.render_frame(cx);
                assert_eq!(divider.read(cx).sizes()[index], minimum);
                window.within(pane).click("explorer-path-display", cx);
                window.render_frame(cx);
                if draft.is_empty() {
                    window.press("backspace", cx);
                } else {
                    window.input(draft, cx);
                }
                window.press("tab", cx);
            })
            .unwrap();
            cx.run_until_parked();
            if index == 0 {
                fs.take_request("/home/test/")
                    .send(Ok((0..20)
                        .map(|index| entry(&format!("file-{index:02}"), EntryKind::File))
                        .collect()))
                    .unwrap();
                cx.run_until_parked();
            }
            cx.update_window(handle, |_, window, cx| {
                for _ in 0..14 {
                    window.press("tab", cx);
                    window.render_frame(cx);
                }
                let selected = window
                    .within(pane)
                    .try_find(("path-suggestion", 14usize))
                    .unwrap_or_else(|| panic!("Tab-selected candidate disappeared from {pane} at {minimum:?} in {mode:?}"));
                assert!(selected.visible(), "{pane} at minimum height in {mode:?}");
                let bounds = selected.bounds();
                // Completion floats below the field and can flip above it in
                // a short pane, while remaining within the window viewport.
                let bottom = window.viewport_size().height;
                assert!(
                    bounds.bottom() <= bottom,
                    "selected candidate must fit {pane}: {bounds:?}"
                );
                assert!(bounds.origin.x >= px(0.) && bounds.origin.y >= px(0.));
                assert!(bounds.right() <= window.viewport_size().width);
                let editor = if index == 0 {
                    panel.read(cx).remote_path.entity_id()
                } else {
                    panel.read(cx).local_path.entity_id()
                };
                let popup = window.find(("path-completion-popup", editor));
                assert!(popup.visible());
                let popup_bounds = popup.bounds();
                assert!(popup_bounds.origin.x >= px(0.) && popup_bounds.origin.y >= px(0.));
                assert!(popup_bounds.right() <= window.viewport_size().width);
                assert!(popup_bounds.bottom() <= window.viewport_size().height);
                window.within(pane).click(("path-suggestion", 14usize), cx);
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
    }
}
