//! Opening remote files from the Explorer in a tab on their host.
use super::*;
use std::cell::RefCell;

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

struct EtcFs;
impl RemoteFs for EtcFs {
    fn home(&self) -> FsFuture<String> {
        async { Ok("/etc".into()) }.boxed()
    }
    fn read_dir(&self, _: &str) -> FsFuture<Vec<DirEntry>> {
        async { Ok(vec![entry("hosts", EntryKind::File)]) }.boxed()
    }
}

#[gpui_kit::test]
fn double_clicking_a_remote_file_opens_the_editor_on_its_host(cx: &mut TestAppContext) {
    let opened = Rc::new(RefCell::new(Vec::<nocterm_workspace::SessionSpec>::new()));
    let record = opened.clone();
    let target = nocterm_session::Target::parse("root@vm", None).unwrap();
    let spec = nocterm_workspace::SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: Some("vm".into()),
        target,
        auth: Default::default(),
        launch: None,
        credential: None,
    };
    let (window, panel) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    workspace
                        .set_session_opener(move |_, spec, _, _| record.borrow_mut().push(spec));
                    workspace
                })
            })
            .unwrap();
        let panel = window
            .update(cx, |_, window, cx| {
                let item = cx.new(|cx| HostItem {
                    spec,
                    fs: Arc::new(EtcFs),
                    focus: cx.focus_handle(),
                });
                workspace.update(cx, |workspace, cx| workspace.add_item(item, window, cx));
                let session = workspace.read(cx).active_session(cx);
                cx.new(|cx| FilesPanel::new(workspace.clone(), session, window, cx))
            })
            .unwrap();
        (window, panel)
    });
    cx.run_until_parked();
    let double_click = gpui_kit::MouseDownEvent {
        position: Default::default(),
        button: MouseButton::Left,
        modifiers: Default::default(),
        click_count: 2,
        first_mouse: false,
    };
    panel.update(cx, |panel, cx| {
        assert_eq!(panel.browser.entries.len(), 1);
        panel.select_remote(0, &double_click, cx);
    });
    cx.run_until_parked();
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1, "one editor tab");
    let launch = opened[0].launch.as_ref().unwrap();
    assert_eq!(launch.program.as_deref(), Some("nano"));
    assert_eq!(launch.args, ["/etc/hosts"]);
    assert_eq!(launch.cwd.as_deref(), Some("/etc"));
    assert_eq!(opened[0].title.as_ref(), "nano hosts");
    let _ = window;
}
