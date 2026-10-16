use super::*;
use nocterm_session::{
    ProgramOpenError,
    testing::{FakeExec, Gate},
};

struct OpenFixture {
    installed: Installed,
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    panel: Entity<FilesPanel>,
    directory: tempfile::TempDir,
    native: Arc<FakeExec>,
}
fn fixture(cx: &mut TestAppContext, name: &std::ffi::OsStr, native: Arc<FakeExec>) -> OpenFixture {
    fixture_with_store(
        cx,
        name,
        native,
        nocterm_ui::SettingsStore::in_memory(Default::default()),
    )
}
fn fixture_with_store(
    cx: &mut TestAppContext,
    name: &std::ffi::OsStr,
    native: Arc<FakeExec>,
    store: nocterm_ui::SettingsStore,
) -> OpenFixture {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(name), b"file").unwrap();
    let (mut deps, _) = fixture_deps_at(directory.path());
    deps.opener = native.clone();
    let settings = ExplorerSettings {
        open: OpenSettings {
            local: Opener::new("editor", &["--literal", "pre={file}:{file}"]),
            ..Default::default()
        },
        ..Default::default()
    };
    let (window, workspace, _, panel, installed) = panel_fixture_with_settings(
        cx,
        Arc::new(PendingFs::default()),
        deps,
        store
            .with(settings)
            .with(nocterm_session::LocalShellSettings::default()),
    );
    cx.update_window(window, |_, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
        panel.update(cx, |panel, cx| {
            panel.navigate_local(directory.path().into(), cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_window_resize(window, gpui_kit::size(px(1400.), px(1100.)));
    OpenFixture {
        installed,
        window,
        workspace,
        panel,
        directory,
        native,
    }
}
fn double_click(cx: &mut TestAppContext, window: AnyWindowHandle) {
    double_click_entry(cx, window, "local-entry");
}
fn notice_count(cx: &mut TestAppContext, window: AnyWindowHandle) -> usize {
    cx.update_window(window, |_, window, cx| {
        nocterm_ui::notice::count(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn configured_local_editor_receives_all_placeholders_as_literal_arguments(cx: &mut TestAppContext) {
    let fixture = fixture(cx, "a b;$.txt".as_ref(), Arc::new(FakeExec::default()));
    double_click(cx, fixture.window);
    let requests = fixture.native.opened_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program(), "editor");
    let file = fixture.directory.path().join("a b;$.txt");
    assert_eq!(
        requests[0].args(),
        [
            "--literal".to_owned(),
            format!("pre={}:{}", file.display(), file.display())
        ]
    );
    assert_eq!(requests[0].cwd(), Some(fixture.directory.path()));
    assert_eq!(notice_count(cx, fixture.window), 0);
}

#[gpui_kit::test]
fn typed_busy_or_spawn_failure_is_reported_in_the_original_window(cx: &mut TestAppContext) {
    let native = Arc::new(FakeExec::default());
    native
        .queue_open(Err(ProgramOpenError::Busy { limit: 32 }))
        .unwrap();
    native
        .queue_open(Err(ProgramOpenError::Start {
            program: "editor".into(),
            source: std::io::ErrorKind::NotFound.into(),
        }))
        .unwrap();
    let fixture = fixture(cx, "note.txt".as_ref(), native);
    double_click(cx, fixture.window);
    assert_eq!(notice_count(cx, fixture.window), 1);
    double_click(cx, fixture.window);
    assert_eq!(fixture.native.opened_requests().len(), 2);
    assert_eq!(
        notice_count(cx, fixture.window),
        1,
        "same operation replaces its notice"
    );
}

struct EmptyRoot;
impl Render for EmptyRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn delayed_launch_error_survives_removal_of_the_explorer_and_workspace(cx: &mut TestAppContext) {
    let native = Arc::new(FakeExec::default());
    let gate = Gate::pending();
    native.suspend_open(gate.clone());
    native
        .queue_open(Err(ProgramOpenError::Busy { limit: 32 }))
        .unwrap();
    let fixture = fixture(cx, "note.txt".as_ref(), native);
    double_click(cx, fixture.window);
    assert_eq!(fixture.native.opened_requests().len(), 1);
    assert_eq!(notice_count(cx, fixture.window), 0);
    cx.update_window(fixture.window, |_, window, cx| {
        window.replace_root(cx, |_, _| EmptyRoot);
    })
    .unwrap();
    drop(fixture.panel);
    drop(fixture.workspace);
    gate.release();
    cx.run_until_parked();
    assert_eq!(notice_count(cx, fixture.window), 1);
}

#[gpui_kit::test]
fn closed_origin_window_never_redirects_delayed_failure_to_another_window(cx: &mut TestAppContext) {
    let native = Arc::new(FakeExec::default());
    let gate = Gate::pending();
    native.suspend_open(gate.clone());
    native
        .queue_open(Err(ProgramOpenError::Busy { limit: 32 }))
        .unwrap();
    let first = fixture(cx, "first.txt".as_ref(), native.clone());
    double_click(cx, first.window);
    let second = fixture(cx, "second.txt".as_ref(), native.clone());
    cx.update_window(first.window, |_, window, _| window.remove_window())
        .unwrap();
    drop(first.panel);
    drop(first.workspace);
    gate.release();
    cx.run_until_parked();
    assert_eq!(native.opened_requests().len(), 1);
    assert_eq!(notice_count(cx, second.window), 0);
}

#[cfg(unix)]
#[gpui_kit::test]
fn native_non_utf8_file_name_reports_before_calling_the_program_port(cx: &mut TestAppContext) {
    use std::os::unix::ffi::OsStrExt as _;
    let fixture = fixture(
        cx,
        std::ffi::OsStr::from_bytes(b"invalid-\xff.txt"),
        Arc::new(FakeExec::default()),
    );
    double_click(cx, fixture.window);
    assert!(fixture.native.opened_requests().is_empty());
    assert_eq!(notice_count(cx, fixture.window), 1);
}

#[gpui_kit::test]
fn menu_snapshot_keeps_its_label_but_dispatch_uses_latest_original_published_policy(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let fixture = fixture(cx, "a b;$.txt".as_ref(), Arc::new(FakeExec::default()));
    crate::tests::operations_ui::context_menu(cx, fixture.window, true, 0);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.within("popup-menu").find(9usize).label(),
            Some("Open with editor")
        );
    })
    .unwrap();
    cx.update(|cx| {
        fixture
            .installed
            .settings()
            .update::<ExplorerSettings>(cx, |settings| {
                settings.open.local =
                    Opener::new("latest editor", &["--literal", "pre={file}:{file}"]);
            })
            .detach();
        let other = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(ExplorerSettings {
                open: OpenSettings {
                    local: Opener::new("wrong owner", &[]),
                    ..Default::default()
                },
                ..Default::default()
            })
            .install(cx);
        cx.set_global(nocterm_ui::SettingsStore::from_handle(other));
    });
    crate::tests::operations_ui::choose(cx, fixture.window, 9);
    let opened = fixture.native.opened_requests();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].program(), "latest editor");
    let file = fixture.directory.path().join("a b;$.txt");
    assert_eq!(
        opened[0].args(),
        [
            "--literal".to_owned(),
            format!("pre={}:{}", file.display(), file.display())
        ]
    );
    assert_eq!(opened[0].cwd(), Some(fixture.directory.path()));
}

#[gpui_kit::test]
async fn rejected_durable_opener_policy_keeps_the_original_published_selection(
    cx: &mut TestAppContext,
) {
    let blocked = tempfile::tempdir().unwrap();
    let store = nocterm_ui::SettingsStore::new(
        Default::default(),
        nocterm_settings::SettingsFile::new(blocked.path()),
    );
    let fixture = fixture_with_store(
        cx,
        "note.txt".as_ref(),
        Arc::new(FakeExec::default()),
        store,
    );
    crate::tests::operations_ui::context_menu(cx, fixture.window, true, 0);
    let result = cx
        .update(|cx| {
            fixture
                .installed
                .settings()
                .update::<ExplorerSettings>(cx, |settings| {
                    settings.open.local = Opener::new("rejected editor", &[]);
                })
        })
        .await;
    assert!(matches!(result, Err(nocterm_store::StoreError::Backend(_))));
    crate::tests::operations_ui::choose(cx, fixture.window, 9);
    let requests = fixture.native.opened_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program(), "editor");
    cx.update(|cx| {
        assert_eq!(
            fixture
                .installed
                .settings()
                .setting::<ExplorerSettings>(cx)
                .open
                .local
                .program,
            "editor"
        )
    });
}
