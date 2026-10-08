//! Uploads, downloads and drags between the panes queue the right transfers.
use super::*;

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn external_folder_drop_pins_context_internal_drag_queues_and_queue_cancel_works(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{FileDropEvent, InputEvent as _, test::TestWindowExt as _};
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.bin");
    std::fs::write(&file, b"queued-data").unwrap();
    let original = Arc::new(StalledUploads::default());
    let replacement = Arc::new(StalledUploads::default());
    let (handle, workspace, item, panel) = panel_fixture(cx, original.clone());
    let service = cx
        .update_window(handle, |_, _, cx| {
            transfers::init(cx);
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            panel.update(cx, |panel, cx| {
                panel.load_local(directory.path().into(), cx)
            });
            transfers::service(cx).unwrap()
        })
        .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1200.), px(900.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert!(!panel.read(cx).browser.loading);
        assert_eq!(panel.read(cx).browser.entries.len(), 1);
        // Native virtual-list rows have no toolkit observation snapshots.
        // Derive the first-row center from the observed toolbar bounds.
        let toolbar = window.find("files-home").bounds();
        let position = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([file.clone()].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        service.snapshot().len(),
        1,
        "folder drop must stop bubbling before the browser's second handler"
    );
    item.update(cx, |item, cx| {
        item.session.target.host = "second-host".into();
        item.session.fs = Some(replacement.clone());
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    wait_upload_started(&service, 1);
    assert_eq!(service.snapshot()[0].target.host, "host");
    assert_eq!(service.snapshot()[0].destination, "/home/test/incoming");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while original.paths.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(replacement.paths.lock().unwrap().is_empty());
    assert_eq!(
        original.paths.lock().unwrap()[0],
        "/home/test/incoming/file.bin"
    );
    let first = service.snapshot()[0].id;
    service.cancel(first);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !service.snapshot()[0].state.finished() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let local = window.find("local-home").bounds();
        let remote = window.find("files-home").bounds();
        window.drag(
            gpui_kit::point(local.center().x, local.bottom() + px(36.)),
            gpui_kit::point(remote.center().x, remote.bottom() + px(36.)),
            cx,
        );
    })
    .unwrap();
    assert_eq!(
        service.snapshot().len(),
        2,
        "internal row drag should enqueue one batch"
    );
    wait_upload_started(&service, 1);
    let current = service
        .snapshot()
        .into_iter()
        .find(|job| !job.state.finished())
        .unwrap();
    assert_eq!(current.target.host, "second-host");
    cx.update_window(handle, |_, window, cx| {
        let queue = cx.new(|cx| transfers::TransfersView::new(workspace.downgrade(), cx));
        workspace.update(cx, |workspace, cx| workspace.add_item(queue, window, cx));
        window.render_frame(cx);
        window.click(
            SharedString::from(format!("cancel-upload-{}", current.id)),
            cx,
        );
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while service.snapshot().iter().any(|job| !job.state.finished()) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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
        window.click(
            SharedString::from(format!("retry-upload-{}", current.id)),
            cx,
        );
    })
    .unwrap();
    assert_eq!(
        service.snapshot().len(),
        3,
        "Opening Uploads must allow explicit Retry against the still-connected matching SSH tab"
    );
    for job in service.snapshot() {
        service.cancel(job.id);
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn remote_selection_download_button_pins_context_and_disconnected_access_is_disabled(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let original = Arc::new(RecordedDownloads::new(true));
    let replacement = Arc::new(RecordedDownloads::new(true));
    let (handle, workspace, item, panel) = panel_fixture(cx, original.clone());
    let service = cx
        .update_window(handle, |_, _, cx| {
            transfers::init(cx);
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            panel.update(cx, |panel, cx| {
                panel.load_local(directory.path().into(), cx)
            });
            transfers::service(cx).unwrap()
        })
        .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("files-download", cx);
        assert!(
            service.snapshot().is_empty(),
            "Download without selection must be disabled"
        );
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
        assert_eq!(
            panel
                .read(cx)
                .remote_selected
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [0, 1, 2]
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
        assert_eq!(
            panel
                .read(cx)
                .remote_selected
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [0, 2]
        );
        window.click("collision-policy-rename", cx);
        assert_eq!(panel.read(cx).collisions, CollisionPolicy::Rename);
        panel.update(cx, |panel, cx| {
            panel.local.path = None;
            cx.notify();
        });
        window.click("files-download", cx);
        assert!(
            service.snapshot().is_empty(),
            "Download without a local destination must be disabled"
        );
        panel.update(cx, |panel, cx| {
            panel.local.path = Some(directory.path().into());
            cx.notify();
        });
        window.click("files-download", cx);
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 1);
    item.update(cx, |item, cx| {
        item.session.target.host = "replacement".into();
        item.session.fs = Some(replacement.clone());
        cx.emit(ItemEvent::Changed);
    });
    panel.update(cx, |panel, cx| panel.load_local(other.clone(), cx));
    cx.run_until_parked();
    wait_transfer(&service, |jobs| {
        jobs[0].discovered_files == 2 && original.paths.lock().unwrap().len() == 2
    });
    let job = service.snapshot().remove(0);
    assert_eq!(job.target.host, "host");
    assert_eq!(
        job.direction,
        nocterm_transfers::TransferDirection::Download
    );
    assert_eq!(job.destination, directory.path().display().to_string());
    assert!(replacement.paths.lock().unwrap().is_empty());
    let mut paths = original.paths.lock().unwrap().clone();
    paths.sort();
    assert_eq!(paths, ["/home/test/a.txt", "/home/test/c.txt"]);
    item.update(cx, |item, cx| {
        item.session.connected = false;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("files-download", cx);
        assert_eq!(
            service.snapshot().len(),
            1,
            "disconnected Download must not enqueue"
        );
    })
    .unwrap();
    item.update(cx, |item, cx| {
        item.session.connected = true;
        item.session.fs = None;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("files-download", cx);
        assert_eq!(
            service.snapshot().len(),
            1,
            "Download without SFTP must not enqueue"
        );
    })
    .unwrap();
    service.cancel(job.id);
    wait_transfer(&service, |jobs| jobs.iter().all(|job| job.state.finished()));
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn remote_drag_to_local_folder_and_area_queues_once_and_collision_buttons_apply(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("incoming");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), b"keep").unwrap();
    let fs = Arc::new(RecordedDownloads::new(false));
    let (handle, workspace, _, panel) = panel_fixture(cx, fs.clone());
    let service = cx
        .update_window(handle, |_, _, cx| {
            transfers::init(cx);
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            panel.update(cx, |panel, cx| {
                panel.load_local(directory.path().into(), cx)
            });
            transfers::service(cx).unwrap()
        })
        .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    for (index, policy) in [(0, "skip"), (1, "rename"), (2, "replace")] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(SharedString::from(format!("collision-policy-{policy}")), cx);
            let remote = window.find("files-home").bounds();
            let local = window.find("local-home").bounds();
            window.drag(
                gpui_kit::point(remote.center().x, remote.bottom() + px(36.)),
                gpui_kit::point(local.center().x, local.bottom() + px(36.)),
                cx,
            );
        })
        .unwrap();
        assert_eq!(
            service.snapshot().len(),
            index + 1,
            "folder handler must stop parent drop bubbling"
        );
        wait_transfer(&service, |jobs| jobs.iter().all(|job| job.state.finished()));
        assert_eq!(
            service.snapshot().last().unwrap().destination,
            folder.display().to_string()
        );
        match policy {
            "skip" => {
                assert_eq!(std::fs::read(folder.join("a.txt")).unwrap(), b"keep");
                assert_eq!(service.snapshot()[0].skipped_files, 1);
            }
            "rename" => {
                assert_eq!(std::fs::read(folder.join("a.txt")).unwrap(), b"keep");
                assert_eq!(std::fs::read(folder.join("a (1).txt")).unwrap(), b"payload");
            }
            "replace" => assert_eq!(std::fs::read(folder.join("a.txt")).unwrap(), b"payload"),
            _ => unreachable!(),
        }
    }
    let existing_ids: std::collections::HashSet<_> =
        service.snapshot().iter().map(|job| job.id).collect();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let remote = window.find("files-home").bounds();
        let local = window.find("local-home").bounds();
        // The Local toolbar exercises the browser handler outside folder rows.
        window.drag(
            gpui_kit::point(remote.center().x, remote.bottom() + px(68.)),
            gpui_kit::point(local.center().x, local.center().y),
            cx,
        );
    })
    .unwrap();
    assert_eq!(service.snapshot().len(), 4);
    wait_transfer(&service, |jobs| jobs.iter().all(|job| job.state.finished()));
    assert_eq!(
        service
            .snapshot()
            .into_iter()
            .find(|job| !existing_ids.contains(&job.id))
            .unwrap()
            .destination,
        directory.path().display().to_string()
    );
    assert_eq!(
        std::fs::read(directory.path().join("b.txt")).unwrap(),
        b"payload"
    );
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn remote_drag_captures_source_before_switching_tabs_and_destination_when_dropped(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
    let directory = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let original = Arc::new(RecordedDownloads::new(true));
    let replacement = Arc::new(RecordedDownloads::new(true));
    let (handle, workspace, item, panel) = panel_fixture(cx, original.clone());
    let service = cx
        .update_window(handle, |_, _, cx| {
            transfers::init(cx);
            workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
            panel.update(cx, |panel, cx| {
                panel.load_local(directory.path().into(), cx)
            });
            transfers::service(cx).unwrap()
        })
        .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let toolbar = window.find("files-home").bounds();
        let origin = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: origin,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position: origin,
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
                position: origin + gpui_kit::point(px(14.), px(0.)),
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    // An existing drag owns its payload even while the active session changes.
    item.update(cx, |item, cx| {
        item.session.target.host = "replacement".into();
        item.session.fs = Some(replacement.clone());
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let destination = window.find("local-home").bounds().center();
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: destination,
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                position: destination,
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
    panel.update(cx, |panel, cx| panel.load_local(other.path().into(), cx));
    cx.run_until_parked();
    wait_transfer(&service, |_| original.paths.lock().unwrap().len() == 1);
    let job = service.snapshot().remove(0);
    assert_eq!(job.target.host, "host");
    assert_eq!(job.destination, directory.path().display().to_string());
    assert_eq!(original.paths.lock().unwrap()[0], "/home/test/a.txt");
    assert!(replacement.paths.lock().unwrap().is_empty());
    service.cancel(job.id);
    wait_transfer(&service, |jobs| jobs.iter().all(|job| job.state.finished()));
}
