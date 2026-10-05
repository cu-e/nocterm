use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, TestAppContext, Window, WindowOptions, div,
    prelude::*,
};
use nocterm_session::{Auth, Target};

use crate::{Item, ItemEvent, SessionContext, SessionSpec, Workspace};

struct Remote {
    focus: FocusHandle,
    spec: SessionSpec,
    connected: bool,
}
impl EventEmitter<ItemEvent> for Remote {}
impl Focusable for Remote {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Remote {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus)
    }
}
impl Item for Remote {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        self.spec.title.clone()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        Some(SessionContext::new(
            self.spec.target.clone(),
            None,
            self.connected,
        ))
    }
    fn session_spec(&self, _: &App) -> Option<SessionSpec> {
        Some(self.spec.clone())
    }
}

#[gpui_kit::test]
fn programs_open_on_a_connected_host_with_its_sign_in(cx: &mut TestAppContext) {
    let target = Target::parse("root@vm", None).unwrap();
    let opened = Rc::new(RefCell::new(Vec::<SessionSpec>::new()));
    let record = opened.clone();
    let spec = SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: Some("profile-1".into()),
        target: target.clone(),
        auth: Auth::Password,
        launch: None,
        credential: None,
    };
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(window, cx);
                workspace.set_session_opener(move |_, spec, _, _| record.borrow_mut().push(spec));
                workspace
            })
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let offline = cx.new(|cx| Remote {
                focus: cx.focus_handle(),
                spec: spec.clone(),
                connected: false,
            });
            workspace.add_item(offline, window, cx);
            let error = workspace
                .open_on_host(&target, "nano".into(), vec![], None, "x".into(), window, cx)
                .unwrap_err();
            assert!(error.contains("No connected session"), "{error}");
            let online = cx.new(|cx| Remote {
                focus: cx.focus_handle(),
                spec: spec.clone(),
                connected: true,
            });
            workspace.add_item(online, window, cx);
            workspace
                .open_on_host(
                    &target,
                    "nvim".into(),
                    vec!["-R".into(), "/etc/hosts".into()],
                    Some("/etc".into()),
                    "hosts".into(),
                    window,
                    cx,
                )
                .unwrap();
        })
    })
    .unwrap();
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1);
    let launch = opened[0].launch.as_ref().unwrap();
    assert_eq!(launch.program.as_deref(), Some("nvim"));
    assert_eq!(launch.args, ["-R", "/etc/hosts"]);
    assert_eq!(launch.cwd.as_deref(), Some("/etc"));
    assert!(!launch.integration);
    assert_eq!(opened[0].title.as_ref(), "hosts");
    assert_eq!(opened[0].profile.as_deref(), Some("profile-1"));
    assert_eq!(opened[0].auth, Auth::Password);
}
