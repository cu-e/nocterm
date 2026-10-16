use super::*;
use futures::channel::oneshot;
use futures::{FutureExt as _, executor::block_on};
use gpui_kit::{AnyWindowHandle, EventEmitter, TestAppContext, WindowOptions};
use nocterm_session::FsFuture;
use nocterm_workspace::{Item, ItemEvent};
use std::sync::Mutex;

fn click_rendered(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(SharedString::from(id.to_owned()), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

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
fn fixture_deps() -> Deps {
    let local = Arc::new(nocterm_session::testing::InMemoryFs::default());
    Deps {
        local: local.clone(),
        mutations: local,
        transfers: Arc::new(nocterm_transfers::testing::FakeTransfers::default()),
        opener: Arc::new(nocterm_session::testing::FakeExec::default()),
    }
}
/// Materializes native test data into the shared port fake; it implements no I/O port.
fn fixture_deps_at(root: &std::path::Path) -> (Deps, Arc<nocterm_session::testing::InMemoryFs>) {
    let local = Arc::new(nocterm_session::testing::InMemoryFs::default());
    seed_fixture_tree(&local, root);
    let deps = Deps {
        local: local.clone(),
        mutations: local.clone(),
        transfers: Arc::new(nocterm_transfers::testing::FakeTransfers::default()),
        opener: Arc::new(nocterm_session::testing::FakeExec::default()),
    };
    (deps, local)
}
fn transfer_fixture_deps_at(
    root: &std::path::Path,
) -> (
    Deps,
    Arc<nocterm_session::testing::InMemoryFs>,
    Arc<nocterm_transfers::testing::FakeTransfers>,
) {
    let (mut deps, local) = fixture_deps_at(root);
    let transfers = Arc::new(nocterm_transfers::testing::FakeTransfers::default());
    deps.transfers = transfers.clone();
    (deps, local, transfers)
}
fn transfer_remote(folder: bool) -> Arc<nocterm_session::testing::InMemoryFs> {
    let fs = Arc::new(nocterm_session::testing::InMemoryFs::default());
    block_on(fs.create_dir("/home")).unwrap();
    block_on(fs.create_dir("/home/test")).unwrap();
    fs.set_home("/home/test").unwrap();
    if folder {
        block_on(fs.create_dir("/home/test/incoming")).unwrap();
    } else {
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs.seed_file(&format!("/home/test/{name}"), b"payload".to_vec())
                .unwrap();
        }
    }
    fs
}
fn recorded_download(
    recording: &nocterm_transfers::testing::FakeTransfers,
    id: nocterm_transfers::TransferId,
) -> nocterm_transfers::DownloadRequest {
    match recording.request(id).unwrap() {
        nocterm_transfers::Request::Download(request) => request,
        nocterm_transfers::Request::Upload(_) => panic!("expected a download request"),
    }
}
fn recorded_upload(
    recording: &nocterm_transfers::testing::FakeTransfers,
    id: nocterm_transfers::TransferId,
) -> nocterm_transfers::UploadRequest {
    match recording.request(id).unwrap() {
        nocterm_transfers::Request::Upload(request) => request,
        nocterm_transfers::Request::Download(_) => panic!("expected an upload request"),
    }
}
fn complete_recorded(
    recording: &nocterm_transfers::testing::FakeTransfers,
    id: nocterm_transfers::TransferId,
) {
    for event in [
        nocterm_transfers::TransferEvent::DiscoveryComplete,
        nocterm_transfers::TransferEvent::WorkersDrained,
    ] {
        recording.advance(id, event).unwrap();
    }
}
fn seed_fixture_tree(local: &nocterm_session::testing::InMemoryFs, path: &std::path::Path) {
    for ancestor in path
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter(|ancestor| ancestor.parent().is_some())
    {
        if ancestor != path {
            local.seed_native_directory(ancestor).unwrap();
        }
    }
    let metadata = std::fs::symlink_metadata(path).unwrap();
    if metadata.is_symlink() {
        let destination = std::fs::read_link(path).unwrap();
        local.seed_native_alias(path, &destination).unwrap();
    } else if metadata.is_dir() {
        local.seed_native_directory(path).unwrap();
        for entry in std::fs::read_dir(path).unwrap() {
            seed_fixture_tree(local, &entry.unwrap().path());
        }
    } else {
        local
            .seed_native_file(path, std::fs::read(path).unwrap())
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            nocterm_session::local_fs::LocalMutations::set_permissions(
                local,
                path,
                metadata.permissions().mode() & 0o7777,
            )
            .unwrap();
        }
    }
}
fn native_bytes(local: &dyn nocterm_session::local_fs::LocalFs, path: &std::path::Path) -> Vec<u8> {
    let mut source = local.open_source(path).unwrap();
    let mut result = Vec::new();
    loop {
        let bytes = source.read(32 * 1024).unwrap();
        if bytes.is_empty() {
            break;
        }
        result.extend(bytes);
    }
    source.close().unwrap();
    result
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
    panel_fixture_with(cx, fs, fixture_deps())
}
fn panel_fixture_at(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
    root: &std::path::Path,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
) {
    panel_fixture_with(cx, fs, fixture_deps_at(root).0)
}
fn panel_fixture_with(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
    deps: Deps,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
) {
    let (window, workspace, item, panel, _) = panel_fixture_with_installation(cx, fs, deps);
    (window, workspace, item, panel)
}
fn panel_fixture_with_installation(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
    deps: Deps,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
    Installed,
) {
    panel_fixture_with_settings(
        cx,
        fs,
        deps,
        nocterm_store::SettingsConfig::in_memory(Default::default()),
    )
}
fn panel_fixture_with_settings(
    cx: &mut TestAppContext,
    fs: Arc<dyn RemoteFs>,
    deps: Deps,
    settings: nocterm_store::SettingsConfig,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<SessionItem>,
    Entity<FilesPanel>,
    Installed,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (settings).install(cx),
            cx,
        );
        let installed = install(deps, &ready, cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        let (item, panel) = window
            .update(cx, |_, window, cx| {
                let item = cx.new(|cx| SessionItem {
                    focus: cx.focus_handle(),
                    session: SessionContext::new(
                        nocterm_session::Target::parse("test@host", None).unwrap(),
                        Some(fs),
                        true,
                    ),
                });
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item(item.clone(), window, cx)
                });
                let session = workspace.read(cx).active_session(cx);
                let panel = cx.new(|cx| {
                    FilesPanel::new(installed.clone(), workspace.clone(), session, window, cx)
                });
                (item, panel)
            })
            .unwrap();
        (window, workspace, item, panel, installed)
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
    panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.remote_listing(cx).names()[0], "current");
        assert!(panel.remote_listing(cx).error().is_none());
        assert!(!panel.remote_listing(cx).loading());
    });
    panel.update(cx, |panel, cx| {
        panel.navigate_remote(Some("/home/test/current".into()), cx)
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
    panel.read_with(cx, |panel, cx| {
        assert!(panel.remote_access(cx) != RemoteAccess::Available);
        assert!(panel.remote_listing(cx).path().is_none());
        assert!(panel.remote_listing(cx).names().is_empty());
        assert!(panel.remote_listing(cx).error().is_none());
        assert!(!panel.remote_listing(cx).loading());
    });
    item.update(cx, |item, cx| {
        item.session.connected = true;
        item.session.fs = None;
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert!(panel.remote_access(cx) == RemoteAccess::Unsupported);
        assert!(
            panel.remote_access(cx) != RemoteAccess::Available,
            "a connected host may lack SFTP"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert!(
            panel.remote_access(cx) == RemoteAccess::NoSession,
            "closing the final tab clears the active session"
        );
        assert!(panel.remote_listing(cx).path().is_none());
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
        panel.navigate_remote(Some("/home/test/docs".into()), cx)
    });
    cx.run_until_parked();
    fs.take_request("/home/test/docs")
        .send(Err(FsError::PermissionDenied {
            path: "/home/test/docs".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/home/test")
        );
        assert!(panel.remote_listing(cx).error().is_some());
        panel.navigate_remote(panel.remote_listing(cx).requested().clone(), cx);
    });
    cx.run_until_parked();
    fs.take_request("/home/test/docs").send(Ok(vec![])).unwrap();
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/home/test/docs")
        );
        assert!(panel.remote_listing(cx).error().is_none());
        assert!(panel.remote_listing(cx).names().is_empty());
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
    ) -> Result<(), nocterm_workspace::LocalDirectoryError> {
        if self.busy {
            return Err(nocterm_workspace::LocalDirectoryError::Busy {
                command: format!(
                    "cd -- {}",
                    nocterm_session::quote_posix(path.to_str().unwrap())
                ),
            });
        }
        self.cwd = path.clone();
        self.accepted.push(path);
        cx.emit(ItemEvent::Changed);
        Ok(())
    }
}

#[gpui_kit::test]
fn local_directory_buttons_sync_only_local_context_and_report_busy_shell(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("one"), b"123").unwrap();
    std::fs::write(second.join("two"), b"4567").unwrap();
    let (handle, workspace, _, panel) = panel_fixture_at(cx, Arc::new(FakeFs), directory.path());
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
            panel.update(cx, |panel, cx| panel.navigate_local(first.clone(), cx));
            shell
        })
        .unwrap();
    cx.run_until_parked();
    click_rendered(cx, handle, "explorer-to-terminal");
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.cwd, first);
        assert_eq!(shell.accepted, vec![first.clone()]);
    });
    panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/home/test")
        );
        assert!(panel.local_listing(cx).error().is_none());
    });
    shell.update(cx, |shell, _| shell.busy = true);
    panel.update(cx, |panel, cx| panel.navigate_local(second.clone(), cx));
    cx.run_until_parked();
    click_rendered(cx, handle, "explorer-to-terminal");
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "busy shell is an operational notice"
        );
    })
    .unwrap();
    panel.read_with(cx, |panel, cx| {
        assert!(panel.local_listing(cx).error().is_none())
    });
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.cwd, first);
        assert_eq!(shell.accepted, vec![first.clone()]);
    });
    shell.update(cx, |shell, cx| {
        shell.cwd = first.clone();
        cx.emit(ItemEvent::Changed);
    });
    cx.run_until_parked();
    click_rendered(cx, handle, "terminal-to-explorer");
    panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.local_listing(cx).path(), Some(first));
        assert_eq!(panel.local_listing(cx).names()[0], "one");
        assert!(panel.local_listing(cx).error().is_none());
        assert_eq!(
            panel.remote_listing(cx).path().as_deref(),
            Some("/home/test")
        );
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
    let (_, _, _, panel) = panel_fixture_at(cx, Arc::new(FakeFs), directory.path());
    panel.update(cx, |panel, cx| {
        panel.navigate_local(first, cx);
        panel.navigate_local(second.clone(), cx);
    });
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.local_listing(cx).path(), Some(second.clone()));
        assert_eq!(panel.local_listing(cx).names()[0], "current");
    });
    let missing = directory.path().join("missing");
    panel.update(cx, |panel, cx| panel.navigate_local(missing.clone(), cx));
    cx.run_until_parked();
    panel.read_with(cx, |panel, cx| {
        assert!(panel.local_listing(cx).error().is_some());
        assert_eq!(panel.local_listing(cx).path(), Some(second));
        assert_eq!(panel.local_listing(cx).requested(), Some(missing));
        assert!(!panel.local_listing(cx).loading());
    });
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
#[path = "listing_ui_tests.rs"]
mod listing_ui;
#[path = "operations_ui_tests.rs"]
mod operations_ui;
#[path = "transfer_ui_tests.rs"]
mod transfer_ui;

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

pub(crate) fn settings_window(settings: nocterm_store::SettingsHandle, cx: &mut gpui_kit::App) {
    settings.register::<crate::ExplorerSettings>(cx);
    gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| crate::settings_page::ExplorerPage::new(settings, window, cx))
    })
    .unwrap();
}
