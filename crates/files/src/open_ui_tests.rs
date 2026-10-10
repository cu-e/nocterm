//! Opening remote files from the Explorer in a tab on their host.
use super::*;
use std::cell::RefCell;

#[path = "open_ui_tests/local.rs"]
mod local;

struct HostItem {
    spec: nocterm_workspace::SessionSpec,
    fs: Arc<dyn RemoteFs>,
    focus: FocusHandle,
}
impl EventEmitter<ItemEvent> for HostItem {}
impl Focusable for HostItem {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for HostItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl Item for HostItem {
    fn tab_title(&self, _: &App) -> SharedString {
        "vm".into()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        Some(SessionContext::new(
            self.spec.target.clone(),
            Some(self.fs.clone()),
            true,
        ))
    }
    fn session_spec(&self, _: &App) -> Option<nocterm_workspace::SessionSpec> {
        Some(self.spec.clone())
    }
}

fn remote_session() -> nocterm_workspace::SessionSpec {
    nocterm_workspace::SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: Some("vm".into()),
        target: nocterm_session::Target::parse("root@vm", None).unwrap(),
        auth: Default::default(),
        launch: None,
        credential: None,
    }
}
fn remote_fixture(
    cx: &mut TestAppContext,
    record: Rc<RefCell<Vec<nocterm_workspace::SessionSpec>>>,
    deps: Deps,
) -> (AnyWindowHandle, Entity<FilesPanel>) {
    let fs = Arc::new(nocterm_session::testing::InMemoryFs::default());
    block_on(fs.create_dir("/etc")).unwrap();
    fs.set_home("/etc").unwrap();
    fs.seed_file("/etc/hosts", b"hosts".to_vec()).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        path_input::install_bindings(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps =
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                    let mut workspace = Workspace::new(deps, window, cx);
                    workspace
                        .set_session_opener(move |_, spec, _, _| record.borrow_mut().push(spec));
                    workspace
                })
            })
            .unwrap();
        let panel = window
            .update(cx, |_, window, cx| {
                let item = cx.new(|cx| HostItem {
                    spec: remote_session(),
                    fs,
                    focus: cx.focus_handle(),
                });
                workspace.update(cx, |workspace, cx| workspace.add_item(item, window, cx));
                let session = workspace.read(cx).active_session(cx);
                let panel =
                    cx.new(|cx| FilesPanel::new(deps, workspace.clone(), session, window, cx));
                workspace.update(cx, |workspace, cx| workspace.add_panel(panel.clone(), cx));
                panel
            })
            .unwrap();
        (window, panel)
    })
}
fn double_click_entry(cx: &mut TestAppContext, handle: AnyWindowHandle, group: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::{InputEvent as _, test::TestWindowExt as _};
        window.activate_window();
        window.render_frame(cx);
        let position = window.find((group, 0usize)).bounds().center();
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 2,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn double_clicking_a_remote_file_opens_the_editor_on_its_host(cx: &mut TestAppContext) {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let native = Arc::new(nocterm_session::testing::FakeExec::default());
    let mut deps = fixture_deps();
    deps.opener = native.clone();
    let (window, _panel) = remote_fixture(cx, opened.clone(), deps);
    cx.run_until_parked();
    cx.simulate_window_resize(window, gpui_kit::size(px(1400.), px(1100.)));
    double_click_entry(cx, window, "remote-entry");
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1, "one editor tab");
    let launch = opened[0].launch.as_ref().unwrap();
    assert_eq!(launch.program.as_deref(), Some("nano"));
    assert_eq!(launch.args, ["/etc/hosts"]);
    assert_eq!(launch.cwd.as_deref(), Some("/etc"));
    assert_eq!(opened[0].title.as_ref(), "nano hosts");
    assert_eq!(opened[0].target, remote_session().target);
    assert!(
        native.opened_requests().is_empty(),
        "remote tabs never launch local editors"
    );
}
