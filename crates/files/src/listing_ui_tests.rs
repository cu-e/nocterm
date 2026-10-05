//! What the Explorer lists: folder statistics, listing failures and fresh
//! listings after transfers.
use super::*;

#[gpui_kit::test]
fn hidden_explorer_reports_partial_statistics_once_and_stale_action_cannot_navigate(
    cx: &mut TestAppContext,
) {
    use {gpui_kit::test::TestWindowExt as _, nocterm_ui::notice};
    let source = tempfile::tempdir().unwrap();
    let current = tempfile::tempdir().unwrap();
    let (handle, _, _, panel) = panel_fixture(cx, Arc::new(PendingFs::default()));
    panel.update(cx, |panel, _| {
        panel.local.path = Some(source.path().into());
        panel.local.requested = panel.local.path.clone();
        *panel.local.counter.progress().lock() = local::Statistics {
            complete: true,
            inaccessible: 1,
            errors: vec!["Permission denied; size is partial.".into()],
            ..Default::default()
        };
    });
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
    panel.update(cx, |panel, _| {
        panel.local.counter.progress().lock().inaccessible = 2
    });
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "same scan emits one warning"
        );
        panel.update(cx, |panel, cx| panel.load_local(current.path().into(), cx));
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
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.requested.as_deref(), Some(current.path()));
        assert_eq!(
            panel.local.path.as_deref(),
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
    let (handle, _, _, panel) = panel_fixture(cx, Arc::new(PendingFs::default()));
    panel.update(cx, |panel, cx| panel.load_local(requested.clone(), cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        window.render_frame(cx);
    })
    .unwrap();
    panel.update(cx, |panel, cx| panel.load_local(requested.clone(), cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "same operation replaces its notice"
        );
    })
    .unwrap();
    std::fs::create_dir(&requested).unwrap();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(notice::run_action(window, cx, "files-local-list"));
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path.as_ref(), Some(&requested));
        assert!(panel.local.error.is_none());
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
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.path.as_deref(), Some("/home/test"));
        assert_eq!(
            statistics::summary(&panel.remote_counter.shown),
            "0 files · 1 folders · home folder size not counted"
        );
    });
    panel.update(cx, |panel, cx| {
        panel.load(Some("/home/test/srv".into()), cx)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            statistics::summary(&panel.remote_counter.shown),
            "1 files · 1 folders · 3.0 KiB"
        );
    });
}

/// Publishes every upload at once and counts listings of each folder.
#[derive(Default)]
struct InstantUploads {
    listings: Mutex<Vec<String>>,
}
impl RemoteFs for InstantUploads {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, directory: &str) -> FsFuture<Vec<DirEntry>> {
        self.listings.lock().unwrap().push(directory.into());
        async { Ok(Vec::new()) }.boxed()
    }
    fn stat(&self, _: &str) -> FsFuture<Option<DirEntry>> {
        async { Ok(None) }.boxed()
    }
    fn upload(
        &self,
        path: &str,
        _: nocterm_session::fs::UploadMode,
    ) -> FsFuture<Box<dyn nocterm_session::fs::RemoteUpload>> {
        let path = path.to_owned();
        async move { Ok(Box::new(Published(path)) as Box<dyn nocterm_session::fs::RemoteUpload>) }
            .boxed()
    }
}
struct Published(String);
impl nocterm_session::fs::RemoteUpload for Published {
    fn write(&mut self, _: Vec<u8>) -> FsFuture<()> {
        async { Ok(()) }.boxed()
    }
    fn finish(self: Box<Self>) -> FsFuture<String> {
        async move { Ok(self.0) }.boxed()
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        async { Ok(()) }.boxed()
    }
}

#[gpui_kit::test]
fn a_finished_upload_lists_the_shown_remote_folder_again(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("report.txt");
    std::fs::write(&file, b"done").unwrap();
    let fs = Arc::new(InstantUploads::default());
    let (handle, _, _, panel) = panel_fixture(cx, fs.clone());
    let service = cx
        .update_window(handle, |_, _, cx| {
            transfers::init(cx);
            transfers::service(cx).unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let listed = |fs: &InstantUploads| {
        fs.listings
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.as_str() == "/home/test")
            .count()
    };
    let before = listed(&fs);
    cx.update_window(handle, |_, window, cx| {
        panel.update(cx, |panel, cx| {
            panel.enqueue(vec![file.clone()], None, window, cx)
        });
    })
    .unwrap();
    wait_transfer(&service, |jobs| jobs.iter().all(|job| job.state.finished()));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(
        service.snapshot()[0].state,
        nocterm_transfers::TransferState::Completed
    );
    assert_eq!(
        listed(&fs),
        before + 1,
        "listed again once, without a click"
    );
}
