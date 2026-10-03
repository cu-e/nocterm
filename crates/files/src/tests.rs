use super::*;
use futures::channel::oneshot;
use futures::{FutureExt as _, executor::block_on};
use gpui_kit::{AnyWindowHandle, EventEmitter, TestAppContext, WindowOptions};
use nocterm_session::FsFuture;
use nocterm_workspace::{Item, ItemEvent};
use std::sync::Mutex;

type ListingReply = oneshot::Sender<Result<Vec<DirEntry>, FsError>>;
#[derive(Default)]
struct PendingFs {
    requests: Mutex<Vec<(String, ListingReply)>>,
}
impl RemoteFs for PendingFs {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, directory: &str) -> FsFuture<Vec<DirEntry>> {
        let (send, receive) = oneshot::channel();
        self.requests.lock().unwrap().push((directory.into(), send));
        async { receive.await.unwrap() }.boxed()
    }
}
impl PendingFs {
    fn take_request(&self, expected: &str) -> ListingReply {
        let (path, reply) = self.requests.lock().unwrap().remove(0);
        assert_eq!(path, expected);
        reply
    }
}
struct SessionItem {
    session: SessionContext,
    focus: FocusHandle,
}
impl EventEmitter<ItemEvent> for SessionItem {}
impl Focusable for SessionItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SessionItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl Item for SessionItem {
    fn tab_title(&self, _: &App) -> SharedString {
        "Test session".into()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        Some(self.session.clone())
    }
}
fn panel_fixture(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(window, cx))
            })
            .unwrap();
        let (item, panel) = window
            .update(cx, |_, window, cx| {
                let item = cx.new(|cx| SessionItem {
                    focus: cx.focus_handle(),
                    session: SessionContext {
                        target: nocterm_session::Target::parse("test@host", None).unwrap(),
                        fs: Some(fs),
                        connected: true,
                    },
                });
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item(item.clone(), window, cx)
                });
                let session = workspace.read(cx).active_session(cx);
                let panel = cx.new(|cx| FilesPanel::new(workspace.clone(), session, cx));
                (item, panel)
            })
            .unwrap();
        (window, workspace, item, panel)
    })
}

#[gpui_kit::test]
fn session_events_ignore_late_requests_and_disable_access_after_disconnect(
    cx: &mut TestAppContext,
) {
    let first = Arc::new(PendingFs::default());
    let second = Arc::new(PendingFs::default());
    let (handle, workspace, item, panel) = panel_fixture(cx, first.clone());
    cx.run_until_parked();
    let late = first.take_request("/home/test");
    item.update(cx, |item, cx| {
        item.session.fs = Some(second.clone());
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    let current = second.take_request("/home/test");
    current
        .send(Ok(vec![entry("current", EntryKind::Directory)]))
        .unwrap();
    cx.run_until_parked();
    assert!(
        late.send(Err(FsError::NotConnected)).is_err(),
        "superseded listing releases its receiver"
    );
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.entries[0].name, "current");
        assert!(panel.browser.error.is_none());
        assert!(!panel.browser.loading);
    });
    panel.update(cx, |panel, cx| {
        panel.load(Some("/home/test/current".into()), cx)
    });
    cx.run_until_parked();
    let late = second.take_request("/home/test/current");
    item.update(cx, |item, cx| {
        item.session.connected = false;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    assert!(
        late.send(Ok(vec![entry("stale", EntryKind::File)]))
            .is_err(),
        "disconnect cancels the pending listing"
    );
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(panel.filesystem().is_none());
        assert!(panel.browser.path.is_none());
        assert!(panel.browser.entries.is_empty());
        assert!(panel.browser.error.is_none());
        assert!(!panel.browser.loading);
    });
    item.update(cx, |item, cx| {
        item.session.connected = true;
        item.session.fs = None;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(panel.session.as_ref().unwrap().connected);
        assert!(
            panel.filesystem().is_none(),
            "a connected host may lack SFTP"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(
            panel.session.is_none(),
            "closing the final tab clears the active session"
        );
        assert!(panel.browser.path.is_none());
    });
}

#[gpui_kit::test]
fn failed_navigation_can_retry_requested_path_and_success_clears_error(cx: &mut TestAppContext) {
    let fs = Arc::new(PendingFs::default());
    let (_, _, _, panel) = panel_fixture(cx, fs.clone());
    cx.run_until_parked();
    fs.take_request("/home/test")
        .send(Ok(vec![entry("docs", EntryKind::Directory)]))
        .unwrap();
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        panel.load(Some("/home/test/docs".into()), cx)
    });
    cx.run_until_parked();
    fs.take_request("/home/test/docs")
        .send(Err(FsError::PermissionDenied {
            path: "/home/test/docs".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        assert_eq!(panel.browser.path.as_deref(), Some("/home/test"));
        assert!(panel.browser.error.is_some());
        panel.load(panel.requested_directory.clone(), cx);
    });
    cx.run_until_parked();
    fs.take_request("/home/test/docs").send(Ok(vec![])).unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.path.as_deref(), Some("/home/test/docs"));
        assert!(panel.browser.error.is_none());
        assert!(panel.browser.entries.is_empty());
    });
}

struct FakeFs;
impl RemoteFs for FakeFs {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, directory: &str) -> FsFuture<Vec<DirEntry>> {
        let directory = directory.to_owned();
        async move {
            if directory == "/denied" {
                return Err(FsError::PermissionDenied { path: directory });
            }
            Ok(vec![
                entry("z.txt", EntryKind::File),
                entry("b", EntryKind::Directory),
                entry("A", EntryKind::Directory),
            ])
        }
        .boxed()
    }
}
fn entry(name: &str, kind: EntryKind) -> DirEntry {
    DirEntry {
        name: name.into(),
        kind,
        is_symlink: false,
        size: None,
    }
}
#[test]
fn home_and_navigation_sort_directories_first() {
    let fs = Arc::new(FakeFs);
    let mut browser = Browser::default();
    let request = browser.begin();
    assert!(browser.finish(request, block_on(listing(fs.clone(), None))));
    assert_eq!(browser.path.as_deref(), Some("/home/test"));
    assert_eq!(
        browser
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "b", "z.txt"]
    );
    let request = browser.begin();
    browser.finish(request, block_on(listing(fs, Some("/home/test/b".into()))));
    assert_eq!(browser.path.as_deref(), Some("/home/test/b"));
}
#[test]
fn permission_error_keeps_previous_directory_for_retry() {
    let mut browser = Browser {
        path: Some("/home/test".into()),
        ..Browser::default()
    };
    let request = browser.begin();
    browser.finish(
        request,
        block_on(listing(Arc::new(FakeFs), Some("/denied".into()))),
    );
    assert!(matches!(
        browser.error,
        Some(FsError::PermissionDenied { .. })
    ));
    assert_eq!(browser.path.as_deref(), Some("/home/test"));
    assert!(!browser.loading);
}
#[test]
fn stale_results_cannot_replace_new_session_or_navigation() {
    let mut browser = Browser::default();
    let old = browser.begin();
    let current = browser.begin();
    assert!(!browser.finish(old, Ok(("/old".into(), vec![]))));
    assert!(browser.loading);
    browser.finish(current, Ok(("/new".into(), vec![])));
    browser.clear();
    assert!(!browser.finish(current, Err(FsError::NotConnected)));
    assert!(browser.path.is_none());
    assert!(browser.error.is_none());
}

struct CwdShell {
    focus: FocusHandle,
    cwd: std::path::PathBuf,
    accepted: Vec<std::path::PathBuf>,
    busy: bool,
}
impl EventEmitter<ItemEvent> for CwdShell {}
impl Focusable for CwdShell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for CwdShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for CwdShell {
    fn tab_title(&self, _: &App) -> SharedString {
        "Local shell".into()
    }
}
impl nocterm_workspace::LocalTerminal for CwdShell {
    fn cwd(&self, _: &App) -> Option<std::path::PathBuf> {
        Some(self.cwd.clone())
    }
    fn change_directory(
        &mut self,
        path: std::path::PathBuf,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.busy {
            return Err("Shell is busy; use an empty prompt.".into());
        }
        self.cwd = path.clone();
        self.accepted.push(path);
        cx.emit(ItemEvent::Changed);
        Ok(())
    }
}

#[gpui_kit::test]
fn local_directory_buttons_sync_only_local_context_and_report_busy_shell(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("one"), b"123").unwrap();
    std::fs::write(second.join("two"), b"4567").unwrap();
    let (handle, workspace, _, panel) = panel_fixture(cx, Arc::new(FakeFs));
    let shell = cx
        .update_window(handle, |_, window, cx| {
            let shell = cx.new(|cx| CwdShell {
                focus: cx.focus_handle(),
                cwd: second.clone(),
                accepted: vec![],
                busy: false,
            });
            workspace.update(cx, |workspace, cx| {
                workspace.add_panel(panel.clone(), cx);
                workspace.set_local_terminal(shell.clone(), window, cx);
            });
            panel.update(cx, |panel, cx| panel.load_local(first.clone(), cx));
            shell
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-to-terminal", cx);
    })
    .unwrap();
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.cwd, first);
        assert_eq!(shell.accepted, vec![first.clone()]);
    });
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.browser.path.as_deref(), Some("/home/test"));
        assert!(panel.local.error.is_none());
    });
    shell.update(cx, |shell, _| shell.busy = true);
    panel.update(cx, |panel, cx| panel.load_local(second.clone(), cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-to-terminal", cx);
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(panel.local.error.as_ref().unwrap().contains("busy"))
    });
    shell.read_with(cx, |shell, _| assert_eq!(shell.cwd, first));
    shell.update(cx, |shell, cx| {
        shell.cwd = first.clone();
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("terminal-to-explorer", cx);
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path, Some(first));
        assert_eq!(panel.local.entries[0].name, "one");
        assert!(panel.local.error.is_none());
        assert_eq!(panel.browser.path.as_deref(), Some("/home/test"));
    });
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.active_session(cx).unwrap().target.host, "host")
    });
}

#[gpui_kit::test]
fn newer_local_navigation_rejects_old_listings_and_keeps_directory_on_error(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("old"), b"old").unwrap();
    std::fs::write(second.join("current"), b"current").unwrap();
    let (_, _, _, panel) = panel_fixture(cx, Arc::new(FakeFs));
    panel.update(cx, |panel, cx| {
        panel.load_local(first, cx);
        panel.load_local(second.clone(), cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert_eq!(panel.local.path, Some(second.clone()));
        assert_eq!(panel.local.entries[0].name, "current");
    });
    let missing = directory.path().join("missing");
    panel.update(cx, |panel, cx| panel.load_local(missing.clone(), cx));
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        assert!(panel.local.error.is_some());
        assert_eq!(panel.local.path, Some(second));
        assert_eq!(panel.local.requested, Some(missing));
        assert!(!panel.local.loading);
    });
}

#[derive(Default)]
struct StalledUploads {
    paths: Mutex<Vec<String>>,
}
impl RemoteFs for StalledUploads {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
        async { Ok(vec![entry("incoming", EntryKind::Directory)]) }.boxed()
    }
    fn stat(&self, _: &str) -> FsFuture<Option<DirEntry>> {
        async { Ok(None) }.boxed()
    }
    fn upload(
        &self,
        path: &str,
        _: nocterm_session::fs::UploadMode,
    ) -> FsFuture<Box<dyn nocterm_session::fs::RemoteUpload>> {
        self.paths.lock().unwrap().push(path.into());
        async { Ok(Box::new(StalledWriter) as Box<dyn nocterm_session::fs::RemoteUpload>) }.boxed()
    }
}
struct StalledWriter;
impl nocterm_session::fs::RemoteUpload for StalledWriter {
    fn write(&mut self, _: Vec<u8>) -> FsFuture<()> {
        async {
            std::future::pending::<()>().await;
            Ok(())
        }
        .boxed()
    }
    fn finish(self: Box<Self>) -> FsFuture<String> {
        async { Err(FsError::Other("unexpected publication".into())) }.boxed()
    }
    fn cancel(self: Box<Self>) -> FsFuture<()> {
        async { Ok(()) }.boxed()
    }
}
fn wait_upload_started(service: &nocterm_transfers::Transfers, count: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while service
        .snapshot()
        .iter()
        .filter(|job| job.state == nocterm_transfers::TransferState::Running)
        .count()
        < count
    {
        assert!(
            std::time::Instant::now() < deadline,
            "uploads did not start: {:?}",
            service.snapshot()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[gpui_kit::test]
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

struct RecordedDownloads {
    paths: Mutex<Vec<String>>,
    stalled: bool,
}
impl RecordedDownloads {
    fn new(stalled: bool) -> Self {
        Self {
            paths: Mutex::new(Vec::new()),
            stalled,
        }
    }
}
impl RemoteFs for RecordedDownloads {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/home/test".into()) }.boxed()
    }
    fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
        async {
            Ok(["a.txt", "b.txt", "c.txt"]
                .into_iter()
                .map(|name| {
                    let mut file = entry(name, EntryKind::File);
                    file.size = Some(7);
                    file
                })
                .collect())
        }
        .boxed()
    }
    fn stat(&self, path: &str) -> FsFuture<Option<DirEntry>> {
        let mut file = entry(nocterm_session::fs::path::file_name(path), EntryKind::File);
        file.size = Some(7);
        async { Ok(Some(file)) }.boxed()
    }
    fn download(&self, path: &str) -> FsFuture<Box<dyn nocterm_session::fs::RemoteDownload>> {
        self.paths.lock().unwrap().push(path.into());
        let reader = RecordedReader {
            data: Some(b"payload".to_vec()),
            stalled: self.stalled,
        };
        async { Ok(Box::new(reader) as Box<dyn nocterm_session::fs::RemoteDownload>) }.boxed()
    }
}
struct RecordedReader {
    data: Option<Vec<u8>>,
    stalled: bool,
}
impl nocterm_session::fs::RemoteDownload for RecordedReader {
    fn read(&mut self, _: usize) -> FsFuture<Vec<u8>> {
        let stalled = self.stalled;
        let data = self.data.take().unwrap_or_default();
        async move {
            if stalled {
                std::future::pending::<()>().await;
            }
            Ok(data)
        }
        .boxed()
    }
    fn close(self: Box<Self>) -> FsFuture<()> {
        async { Ok(()) }.boxed()
    }
}
fn pointer_click(
    window: &mut Window,
    point: gpui_kit::Point<gpui_kit::Pixels>,
    modifiers: gpui_kit::Modifiers,
    cx: &mut App,
) {
    use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
    window.dispatch_event(
        gpui_kit::MouseDownEvent {
            button: MouseButton::Left,
            position: point,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(
        gpui_kit::MouseUpEvent {
            button: MouseButton::Left,
            position: point,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}
fn wait_transfer(
    service: &nocterm_transfers::Transfers,
    predicate: impl Fn(&[nocterm_transfers::Progress]) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = service.snapshot();
        if predicate(&snapshot) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "transfer did not reach expected state: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui_kit::test]
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

#[path = "operations_ui_tests.rs"]
mod operations_ui;

#[test]
fn custom_remote_adapter_cannot_deliver_an_unbounded_explorer_listing() {
    struct OversizedFs {
        names: bool,
    }
    impl RemoteFs for OversizedFs {
        fn home(&self) -> FsFuture<String> {
            async { Ok("/home/test".into()) }.boxed()
        }
        fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
            let entries = if self.names {
                vec![entry(
                    &"x".repeat(local::MAX_DIRECTORY_NAME_BYTES + 1),
                    EntryKind::File,
                )]
            } else {
                vec![entry("file", EntryKind::File); local::MAX_DIRECTORY_ENTRIES + 1]
            };
            async { Ok(entries) }.boxed()
        }
    }
    for names in [false, true] {
        let result = block_on(listing(Arc::new(OversizedFs { names }), None));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Explorer listing limit")
        );
    }
}
