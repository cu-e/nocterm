//! Drag the real local/remote rows and retain their footprint and whole payload.
use super::super::*;
use gpui_kit::{
    ElementId, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point,
    component::{Theme, ThemeMode},
    point,
    test::TestWindowExt as _,
};

fn begin_drag(
    id: impl Into<ElementId>,
    modifiers: Modifiers,
    window: &mut Window,
    cx: &mut App,
) -> (gpui_kit::Bounds<Pixels>, Point<Pixels>) {
    window.render_frame(cx);
    let id = id.into();
    let source = window.find(id.clone()).bounds();
    let source_children = ["explorer-row-name", "explorer-row-icon"]
        .map(|child| window.within(id.clone()).find(child).bounds());
    let origin = source.origin + point(px(28.), px(16.));
    window.dispatch_event(
        MouseMoveEvent {
            position: origin,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseDownEvent {
            position: origin,
            button: MouseButton::Left,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    let position = origin + point(px(18.), px(0.));
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: Some(MouseButton::Left),
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    assert!(cx.has_active_drag());
    let preview = window.find("drag-preview").bounds();
    assert_eq!(preview.size, source.size);
    for (child, source_child) in ["explorer-row-name", "explorer-row-icon"]
        .into_iter()
        .zip(source_children)
    {
        let preview_child = window.within("drag-preview").find(child);
        assert!(preview_child.visible());
        assert_eq!(preview_child.bounds().size, source_child.size);
        assert_eq!(
            preview_child.bounds().origin - preview.origin,
            source_child.origin - source.origin
        );
    }
    (source, position)
}

fn release(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    assert!(!cx.has_active_drag());
}

#[gpui_kit::test]
fn native_local_and_remote_previews_keep_full_rows_and_symbolic_link_names(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let local_name = format!("{}資料.txt", "long-file-name-".repeat(8));
    std::fs::create_dir(directory.path().join("directory")).unwrap();
    std::fs::write(directory.path().join(&local_name), b"content").unwrap();
    let fs = Arc::new(PendingFs::default());
    let (deps, local) = fixture_deps_at(directory.path());
    let (handle, workspace, _, panel) = panel_fixture_with(cx, fs.clone(), deps);
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.path().into(), cx)
        });
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    let remote_name = format!("{}日本語.txt", "remote-link-".repeat(8));
    let mut link = entry(&remote_name, EntryKind::File);
    link.is_symlink = true;
    fs.take_request("/home/test")
        .send(Ok(vec![
            entry("remote-directory", EntryKind::Directory),
            link,
        ]))
        .unwrap();
    cx.run_until_parked();
    let target = directory.path().parent().unwrap().join("preview-target");
    local
        .seed_native_file(&target, b"content".to_vec())
        .unwrap();
    nocterm_session::local_fs::LocalMutations::remove(
        local.as_ref(),
        &directory.path().join(&local_name),
        Default::default(),
        &Default::default(),
    )
    .unwrap();
    local
        .seed_native_alias(&directory.path().join(&local_name), &target)
        .unwrap();
    panel.update(cx, |panel, cx| {
        panel.navigate_local(directory.path().into(), cx)
    });
    cx.run_until_parked();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update_window(handle, |_, window, cx| {
            Theme::change(mode, None, cx);
            window.render_frame(cx);
            for index in 0..panel.read(cx).local_listing(cx).names().len() {
                let (_, position) =
                    begin_drag(("local-entry", index), Default::default(), window, cx);
                release(position, window, cx);
            }
            for index in 0..panel.read(cx).remote_listing(cx).names().len() {
                let (_, position) =
                    begin_drag(("remote-entry", index), Default::default(), window, cx);
                release(position, window, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn native_remote_multiselection_preview_keeps_grabbed_name_and_transfers_both_files(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let fs = transfer_remote(false);
    let (deps, _local, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, _, panel) = panel_fixture_with(cx, fs.clone(), deps);
    let service = cx
        .update_window(handle, |_, _, cx| {
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            panel.update(cx, |panel, cx| {
                panel.navigate_local(directory.path().into(), cx)
            });
            service.clone()
        })
        .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let first = window.find(("remote-entry", 0usize)).bounds().center();
        let third = window.find(("remote-entry", 2usize)).bounds().center();
        pointer_click(window, first, Default::default(), cx);
        pointer_click(
            window,
            third,
            Modifiers {
                control: true,
                ..Default::default()
            },
            cx,
        );
        let (_, _) = begin_drag(
            ("remote-entry", 2usize),
            Modifiers {
                shift: true,
                ..Default::default()
            },
            window,
            cx,
        );
        let destination = window.find("local-home").bounds().center();
        window.dispatch_event(
            MouseMoveEvent {
                position: destination,
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        release(destination, window, cx);
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 1, "One drag enqueues one batch");
    let request = recorded_download(&recording, service.snapshot()[0].id);
    assert_eq!(request.sources, ["/home/test/a.txt", "/home/test/c.txt"]);
    assert_eq!(request.local_destination, directory.path());
    let pinned: Arc<dyn RemoteFs> = fs;
    assert!(Arc::ptr_eq(&request.fs, &pinned));
}
