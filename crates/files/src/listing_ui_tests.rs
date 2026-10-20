//! What the Explorer lists: folder statistics, listing failures and fresh
//! listings after transfers.
use super::*;

#[path = "drag_ui_tests.rs"]
mod drag_ui;

#[path = "path_ui_tests.rs"]
mod path_ui;

#[gpui_kit::test]
fn hidden_explorer_reports_partial_statistics_once_and_stale_action_cannot_navigate(
    cx: &mut TestAppContext,
) {
    use {gpui_kit::test::TestWindowExt as _, nocterm_ui::notice};
    let source = tempfile::tempdir().unwrap();
    let current = tempfile::tempdir().unwrap();
    let (deps, local) = fixture_deps_at(source.path());
    local.seed_native_directory(current.path()).unwrap();
    local
        .seed_native_directory(&source.path().join("locked"))
        .unwrap();
    local.fail_next_local(
        nocterm_session::testing::FsOperation::List,
        source.path().join("locked"),
        nocterm_session::local_fs::LocalFsError::PermissionDenied(source.path().join("locked")),
    );
    let (handle, _, _, panel) = panel_fixture_with(cx, Arc::new(PendingFs::default()), deps);
    panel.update(cx, |panel, cx| {
        panel.navigate_local(source.path().into(), cx)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "hidden Panel still publishes warning"
        );
        assert!(
            window.try_find("local-browser").is_none(),
            "Explorer is not rendered in fixture"
        );
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "same scan emits one warning"
        );
        panel.update(cx, |panel, cx| {
            panel.navigate_local(current.path().into(), cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(notice::run_action(window, cx, "files-partial-statistics"));
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).requested().as_deref(),
            Some(current.path())
        );
        assert_eq!(
            panel.local_listing(cx).path().as_deref(),
            Some(current.path()),
            "stale Recalculate cannot restore an old folder"
        );
    });
}

#[gpui_kit::test]
fn local_listing_failure_posts_deduplicated_retry_for_the_requested_directory(
    cx: &mut TestAppContext,
) {
    use {gpui_kit::test::TestWindowExt as _, nocterm_ui::notice};
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("missing");
    let (deps, local) = fixture_deps_at(directory.path());
    let (handle, _, _, panel) = panel_fixture_with(cx, Arc::new(PendingFs::default()), deps);
    panel.update(cx, |panel, cx| panel.navigate_local(requested.clone(), cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        window.render_frame(cx);
    })
    .unwrap();
    panel.update(cx, |panel, cx| panel.navigate_local(requested.clone(), cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "same operation replaces its notice"
        );
    })
    .unwrap();
    local.seed_native_directory(&requested).unwrap();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(notice::run_action(window, cx, "files-local-list"));
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.local_listing(cx).path().as_ref(), Some(&requested));
        assert!(panel.local_listing(cx).error().is_none());
    });
}

struct SizedFs;
impl RemoteFs for SizedFs {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, directory: &str) -> FsFuture<Vec<DirEntry>> {
        let entries = match directory {
            "/home/test" => vec![entry("srv", EntryKind::Directory)],
            "/home/test/srv" => vec![
                DirEntry {
                    size: Some(2048),
                    ..entry("data.bin", EntryKind::File)
                },
                entry("logs", EntryKind::Directory),
            ],
            _ => vec![DirEntry {
                size: Some(1024),
                ..entry("app.log", EntryKind::File)
            }],
        };
        async move { Ok(entries) }.boxed()
    }
}

#[gpui_kit::test]
fn remote_folders_are_counted_except_the_home_folder(cx: &mut TestAppContext) {
    let (_, _, _, panel) = panel_fixture(cx, Arc::new(SizedFs));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/home/test")
        );
        assert_eq!(
            panel.remote_listing(cx).statistics(),
            "0 files · 1 folders · home folder size not counted"
        );
    });
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/home/test/srv".into()), cx)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).statistics(),
            "1 files · 1 folders · 3.0 KiB"
        );
    });
}

/// Publishes every upload at once and counts listings of each folder.
#[gpui_kit::test]
fn a_finished_upload_lists_the_shown_remote_folder_again(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("report.txt");
    std::fs::write(&file, b"done").unwrap();
    let fs = transfer_remote(false);
    fs.script_listings();
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, _, _, panel) = panel_fixture_with(cx, fs.clone(), deps);
    cx.run_until_parked();
    let initial = fs.take_pending_listing().unwrap();
    assert_eq!(initial.path(), "/home/test");
    initial.send(Ok(Vec::new())).unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        panel.update(cx, |panel, cx| {
            panel.intent(
                &PaneIntent::Upload {
                    sources: vec![file.clone()],
                    destination: None,
                },
                cx,
            )
        });
    })
    .unwrap();
    let id = service.snapshot()[0].id;
    for event in [
        nocterm_transfers::TransferEvent::DiscoveredFile { bytes: 4 },
        nocterm_transfers::TransferEvent::FileCompleted,
    ] {
        recording.advance(id, event).unwrap();
    }
    complete_recorded(&recording, id);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(
        service.snapshot()[0].state,
        nocterm_transfers::TransferState::Completed
    );
    let refreshed = fs.take_pending_listing().unwrap();
    assert_eq!(
        refreshed.path(),
        "/home/test",
        "Finished upload refreshes the visible folder"
    );
    refreshed.send(Ok(Vec::new())).unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(
        !fs.has_pending_listings(),
        "One completion triggers one refresh"
    );
}

#[path = "open_ui_tests.rs"]
mod open_ui;

#[gpui_kit::test]
fn empty_directory_upload_refreshes_the_shown_remote_destination(cx: &mut TestAppContext) {
    use gpui_kit::{FileDropEvent, InputEvent as _, test::TestWindowExt as _};
    let directory = tempfile::tempdir().unwrap();
    let empty = directory.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let fs = transfer_remote(false);
    fs.script_listings();
    let (deps, _, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, _, panel) = panel_fixture_with(cx, fs.clone(), deps);
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    fs.take_pending_listing()
        .unwrap()
        .send(Ok(Vec::new()))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let toolbar = window.find("files-home").bounds();
        let position = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([empty.clone()].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    })
    .unwrap();
    let id = service.snapshot()[0].id;
    assert_eq!(recorded_upload(&recording, id).sources, [empty]);
    block_on(fs.create_dir("/home/test/empty")).unwrap();
    recording
        .advance(id, nocterm_transfers::TransferEvent::DirectoryPrepared)
        .unwrap();
    complete_recorded(&recording, id);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let progress = service.snapshot().remove(0);
    assert_eq!(progress.state, nocterm_transfers::TransferState::Completed);
    assert_eq!(progress.completed_files, 0);
    let refreshed = fs
        .take_pending_listing()
        .expect("Empty-directory upload refreshes its destination");
    assert_eq!(refreshed.path(), "/home/test");
    refreshed
        .send(Ok(vec![entry("empty", EntryKind::Directory)]))
        .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.remote_listing(cx).names(), ["empty"])
    });
}

#[gpui_kit::test]
fn empty_directory_download_refreshes_the_shown_local_destination(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let fs = transfer_remote(true);
    let (deps, local, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, _, panel) = panel_fixture_with(cx, fs, deps);
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.path().to_owned(), cx)
        });
    })
    .unwrap();
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1000.)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let toolbar = window.find("files-home").bounds();
        let position = gpui_kit::point(toolbar.center().x, toolbar.bottom() + px(36.));
        pointer_click(window, position, Default::default(), cx);
        window.render_frame(cx);
        window.click("files-download", cx);
    })
    .unwrap();
    let id = service.snapshot()[0].id;
    assert_eq!(
        recorded_download(&recording, id).sources,
        ["/home/test/incoming"]
    );
    local
        .seed_native_directory(&directory.path().join("incoming"))
        .unwrap();
    recording
        .advance(id, nocterm_transfers::TransferEvent::DirectoryPrepared)
        .unwrap();
    complete_recorded(&recording, id);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(
        service.snapshot()[0].state,
        nocterm_transfers::TransferState::Completed
    );
    assert_eq!(service.snapshot()[0].completed_files, 0);
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).names(),
            ["incoming"],
            "Empty-directory download refreshes its destination"
        );
    });
}

#[gpui_kit::test]
fn acknowledged_directory_download_refreshes_after_later_failure(cx: &mut TestAppContext) {
    partial_directory_download_refreshes(cx, false);
}
#[gpui_kit::test]
fn acknowledged_directory_download_refreshes_after_later_cancellation(cx: &mut TestAppContext) {
    partial_directory_download_refreshes(cx, true);
}
fn partial_directory_download_refreshes(cx: &mut TestAppContext, cancelled: bool) {
    use nocterm_transfers::{
        TransferCause, TransferEvent, TransferIssue, TransferOperation, TransferState,
    };
    let directory = tempfile::tempdir().unwrap();
    let fs = transfer_remote(true);
    let (deps, local, recording) = transfer_fixture_deps_at(directory.path());
    let service = deps.transfers.clone();
    let (handle, workspace, _, panel) = panel_fixture_with(cx, fs.clone(), deps);
    cx.update_window(handle, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.path().to_owned(), cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let id = service
        .enqueue_download(nocterm_transfers::DownloadRequest {
            sources: vec!["/home/test/incoming".into()],
            host: nocterm_session::HostAddress::direct(nocterm_session::Target::new(
                "test", "host", 22,
            )),
            local_destination: directory.path().to_owned(),
            fs,
            collisions: CollisionPolicy::Skip,
        })
        .unwrap();
    local
        .seed_native_directory(&directory.path().join("incoming"))
        .unwrap();
    recording
        .advance(id, TransferEvent::DirectoryPrepared)
        .unwrap();
    if cancelled {
        service.cancel(id).unwrap();
    } else {
        recording
            .advance(
                id,
                TransferEvent::FileFailed(TransferIssue::new(
                    None,
                    TransferOperation::Discover,
                    TransferCause::InvalidName,
                )),
            )
            .unwrap();
        complete_recorded(&recording, id);
    }
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let progress = service.snapshot().remove(0);
    assert_eq!(
        progress.state,
        if cancelled {
            TransferState::Cancelled
        } else {
            TransferState::Failed
        }
    );
    assert_eq!(progress.prepared_directories, 1);
    assert_eq!(progress.completed_files, 0);
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).names(),
            ["incoming"],
            "An acknowledged directory remains visible after later failure/cancellation"
        );
    });
}

fn wait_for_home_reply(observed: &std::sync::atomic::AtomicBool, cx: &mut TestAppContext) {
    for _ in 0..1000 {
        if observed.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        assert!(cx.executor().tick(), "home query must be scheduled");
    }
    panic!("native home query did not execute");
}

#[gpui_kit::test]
fn delayed_home_keeps_the_original_initial_directory_after_published_policy_changes(
    cx: &mut TestAppContext,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let original = tempfile::tempdir().unwrap();
    let later = tempfile::tempdir().unwrap();
    let (deps, local) = fixture_deps_at(original.path());
    local.seed_native_directory(later.path()).unwrap();
    let queried = Arc::new(AtomicBool::new(false));
    let observed = queried.clone();
    local.observe_local(Arc::new(move |operation, _: &std::path::Path| {
        if operation == nocterm_session::testing::FsOperation::Home {
            observed.store(true, Ordering::SeqCst);
        }
    }));
    let store = nocterm_store::SettingsConfig::in_memory(Default::default()).with(
        nocterm_session::LocalShellSettings(nocterm_session::ShellSettings {
            cwd: Some(original.path().to_string_lossy().into_owned()),
            ..Default::default()
        }),
    );
    let (_, _, _, panel, installed) =
        panel_fixture_with_settings(cx, Arc::new(PendingFs::default()), deps, store);
    wait_for_home_reply(&queried, cx);
    panel.read_with(cx, |panel, cx| {
        assert!(panel.local_listing(cx).path().is_none())
    });
    cx.update(|cx| {
        let later_path = later.path().to_string_lossy().into_owned();
        installed
            .settings()
            .update::<nocterm_session::LocalShellSettings>(cx, move |s| {
                s.cwd = Some(later_path.clone());
            })
            .detach();
        let other = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(nocterm_session::LocalShellSettings(
                nocterm_session::ShellSettings {
                    cwd: Some(later.path().to_string_lossy().into_owned()),
                    ..Default::default()
                },
            ))
            .install(cx);
        crate::tests::settings_window(other, cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).path().as_deref(),
            Some(original.path())
        )
    });
}

#[gpui_kit::test]
fn delayed_home_cannot_replace_navigation_accepted_before_its_completion(cx: &mut TestAppContext) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let original = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let (deps, local) = fixture_deps_at(original.path());
    local.seed_native_directory(destination.path()).unwrap();
    let queried = Arc::new(AtomicBool::new(false));
    let observed = queried.clone();
    local.observe_local(Arc::new(move |operation, _: &std::path::Path| {
        if operation == nocterm_session::testing::FsOperation::Home {
            observed.store(true, Ordering::SeqCst);
        }
    }));
    let (_, _, _, panel) = panel_fixture_with(cx, Arc::new(PendingFs::default()), deps);
    wait_for_home_reply(&queried, cx);
    panel.read_with(cx, |panel, cx| {
        assert!(panel.local_listing(cx).path().is_none())
    });
    panel.update(cx, |panel, cx| {
        panel.navigate_local(destination.path().to_owned(), cx)
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.local_listing(cx).requested().as_deref(),
            Some(destination.path())
        );
        assert_eq!(
            panel.local_listing(cx).path().as_deref(),
            Some(destination.path())
        );
    });
}

#[gpui_kit::test]
fn each_indexing_request_reads_latest_original_policy_after_an_unrelated_installation(
    cx: &mut TestAppContext,
) {
    let (_, _, _, panel, installed) =
        panel_fixture_with_installation(cx, Arc::new(SizedFs), fixture_deps());
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/home/test/srv".into()), cx)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).statistics(),
            "1 files · 1 folders · 3.0 KiB"
        )
    });
    cx.update(|cx| {
        installed
            .settings()
            .update::<ExplorerSettings>(cx, |s| s.indexing.remote = false)
            .detach();
        let other = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(ExplorerSettings::default())
            .install(cx);
        crate::tests::settings_window(other, cx);
    });
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/home/test/srv".into()), cx)
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).statistics(),
            "1 files · 1 folders · size not counted"
        )
    });
}
