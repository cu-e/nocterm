//! Transfer UI emits pinned requests; streaming belongs to adapter integration tests.
use super::*;

fn show_local(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    panel: &Entity<FilesPanel>,
    directory: &std::path::Path,
) {
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.to_owned(), cx)
        });
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
}
fn drag_row(window: &mut Window, source: &str, destination: &str, offset: f32, cx: &mut App) {
    use gpui_kit::test::TestWindowExt as _;
    let source = window.find(SharedString::from(source.to_owned())).bounds();
    let destination = window
        .find(SharedString::from(destination.to_owned()))
        .bounds();
    window.drag(
        gpui_kit::point(source.center().x, source.bottom() + px(offset)),
        gpui_kit::point(destination.center().x, destination.bottom() + px(36.)),
        cx,
    );
}
fn drop_external_on_remote(window: &mut Window, file: &std::path::Path, cx: &mut App) {
    use gpui_kit::{FileDropEvent, InputEvent as _, test::TestWindowExt as _};
    window.render_frame(cx);
    window.render_frame(cx);
    let toolbar = window.find("files-home").bounds();
    let position = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
    window.dispatch_event(
        FileDropEvent::Entered {
            position,
            paths: ExternalPaths([file.to_owned()].into_iter().collect()),
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    window.render_frame(cx);
}

fn select_nonadjacent_remote_rows(window: &mut Window, cx: &mut App) {
    use gpui_kit::test::TestWindowExt as _;
    let toolbar = window.find("files-home").bounds();
    let first = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
    pointer_click(window, first, Default::default(), cx);
    pointer_click(
        window,
        first + gpui_kit::point(px(0.), px(64.)),
        gpui_kit::Modifiers {
            shift: true,
            ..Default::default()
        },
        cx,
    );
    pointer_click(
        window,
        first + gpui_kit::point(px(0.), px(32.)),
        gpui_kit::Modifiers {
            control: true,
            ..Default::default()
        },
        cx,
    );
}

#[gpui_kit::test]
fn external_folder_drop_pins_context_internal_drag_queues_and_queue_cancel_works(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.bin");
    std::fs::write(&file, b"queued-data").unwrap();
    let original = transfer_remote(true);
    let replacement = transfer_remote(true);
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, item, panel) = panel_fixture_with(cx, original.clone(), deps);
    show_local(cx, handle, &workspace, &panel, directory.path());
    cx.update_window(handle, |_, window, cx| {
        drop_external_on_remote(window, &file, cx);
    })
    .unwrap();
    assert_eq!(
        service.snapshot().len(),
        1,
        "Folder drop stops parent bubbling"
    );
    let first = service.snapshot()[0].id;
    let request = recorded_upload(&recording, first);
    assert_eq!(request.sources, [file]);
    assert_eq!(request.destination, "/home/test/incoming");
    let pinned: Arc<dyn RemoteFs> = original;
    assert!(Arc::ptr_eq(&request.fs, &pinned));
    item.update(cx, |item, cx| {
        item.session.target.host = "second-host".into();
        item.session.fs = Some(replacement);
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    service.cancel(first).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        drag_row(window, "local-home", "files-home", 36., cx);
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 2);
    let current = service.snapshot()[0].id;
    assert_eq!(
        recorded_upload(&recording, current).host.target.host,
        "second-host"
    );
    cx.update_window(handle, |_, window, cx| {
        let queue =
            cx.new(|cx| transfers::TransfersView::new(service.clone(), workspace.downgrade(), cx));
        workspace.update(cx, |workspace, cx| workspace.add_item(queue, window, cx));
        window.render_frame(cx);
        window.click(SharedString::from(format!("cancel-upload-{current}")), cx);
    })
    .unwrap();
    assert!(
        service
            .snapshot()
            .iter()
            .all(|job| job.state == nocterm_transfers::TransferState::Cancelled)
    );
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(SharedString::from(format!("retry-upload-{current}")), cx);
    })
    .unwrap();
    assert_eq!(
        service.snapshot().len(),
        3,
        "Explicit Retry uses a matching connected session"
    );
}
#[gpui_kit::test]
fn remote_selection_download_button_pins_context_and_disconnected_access_is_disabled(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let original = transfer_remote(false);
    let replacement = transfer_remote(false);
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, item, panel) = panel_fixture_with(cx, original.clone(), deps);
    show_local(cx, handle, &workspace, &panel, directory.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("files-download", cx);
        assert!(service.snapshot().is_empty());
        select_nonadjacent_remote_rows(window, cx);
        assert_eq!(panel.read(cx).remote_listing(cx).selected(), [0, 2]);
        window.click("collision-policy-rename", cx);
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.path().to_owned(), cx)
        });
        window.render_frame(cx);
        window.click("files-download", cx);
        assert!(
            service.snapshot().is_empty(),
            "Pending navigation is not an implicit destination"
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("files-download", cx);
    })
    .unwrap();
    let request = recorded_download(&recording, service.snapshot()[0].id);
    assert_eq!(request.sources, ["/home/test/a.txt", "/home/test/c.txt"]);
    assert_eq!(request.local_destination, directory.path());
    assert_eq!(
        request.collisions,
        nocterm_transfers::CollisionPolicy::Rename
    );
    let pinned: Arc<dyn RemoteFs> = original;
    assert!(Arc::ptr_eq(&request.fs, &pinned));
    item.update(cx, |item, cx| {
        item.session.target.host = "replacement".into();
        item.session.fs = Some(replacement);
        cx.emit(ItemEvent::Changed);
    });
    panel.update(cx, |panel, cx| panel.navigate_local(other.clone(), cx));
    cx.run_until_parked();
    assert_eq!(
        recorded_download(&recording, service.snapshot()[0].id).local_destination,
        directory.path()
    );
    for unsupported in [false, true] {
        item.update(cx, |item, cx| {
            item.session.connected = unsupported;
            if unsupported {
                item.session.fs = None;
            }
            cx.emit(ItemEvent::Changed);
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("files-download", cx);
            assert_eq!(service.snapshot().len(), 1);
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn remote_drag_to_local_folder_and_area_queues_once_and_collision_buttons_apply(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("incoming");
    std::fs::create_dir(&folder).unwrap();
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, _, panel) = panel_fixture_with(cx, transfer_remote(false), deps);
    show_local(cx, handle, &workspace, &panel, directory.path());
    for (index, label, policy) in [
        (0, "skip", nocterm_transfers::CollisionPolicy::Skip),
        (1, "rename", nocterm_transfers::CollisionPolicy::Rename),
        (2, "replace", nocterm_transfers::CollisionPolicy::Replace),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(SharedString::from(format!("collision-policy-{label}")), cx);
            drag_row(window, "files-home", "local-home", 36., cx);
        })
        .unwrap();
        assert_eq!(
            service.snapshot().len(),
            index + 1,
            "Folder drop stops parent bubbling"
        );
        let id = service.snapshot()[0].id;
        let request = recorded_download(&recording, id);
        assert_eq!(request.local_destination, folder);
        assert_eq!(request.collisions, policy);
        complete_recorded(&recording, id);
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let remote = window.find("files-home").bounds();
        let local = window.find("local-home").bounds();
        window.drag(
            gpui_kit::point(remote.center().x, remote.bottom() + px(68.)),
            local.center(),
            cx,
        );
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 4);
    assert_eq!(
        recorded_download(&recording, service.snapshot()[0].id).local_destination,
        directory.path()
    );
}
fn begin_remote_drag(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let toolbar = window.find("files-home").bounds();
        let position = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: position + gpui_kit::point(px(14.), px(0.)),
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn remote_drag_captures_source_before_switching_tabs_and_destination_when_dropped(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
    let directory = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let original = transfer_remote(false);
    let replacement = transfer_remote(false);
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, item, panel) = panel_fixture_with(cx, original.clone(), deps);
    show_local(cx, handle, &workspace, &panel, directory.path());
    begin_remote_drag(cx, handle);
    item.update(cx, |item, cx| {
        item.session.target.host = "replacement".into();
        item.session.fs = Some(replacement);
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find("local-home").bounds().center();
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position,
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 1);
    let request = recorded_download(&recording, service.snapshot()[0].id);
    let pinned: Arc<dyn RemoteFs> = original;
    assert!(Arc::ptr_eq(&request.fs, &pinned));
    assert_eq!(request.host.target.host, "host");
    assert_eq!(request.sources, ["/home/test/a.txt"]);
    panel.update(cx, |panel, cx| {
        panel.navigate_local(other.path().to_owned(), cx)
    });
    cx.run_until_parked();
    assert_eq!(request.local_destination, directory.path());
}
