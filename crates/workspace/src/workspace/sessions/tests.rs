use std::{cell::RefCell, rc::Rc};

use gpui_kit::{TestAppContext, WindowOptions, prelude::*};
use nocterm_session::{Auth, Target};

use crate::{SessionSpec, Workspace};

use crate::test_support::SessionProbe as Remote;

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
                let deps = crate::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                let mut workspace = Workspace::new(deps, window, cx);
                workspace.set_session_opener(move |_, spec, _, _| record.borrow_mut().push(spec));
                workspace
            })
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let offline = Remote::new(spec.clone(), false, cx);
            workspace.add_item(offline, window, cx);
            let error = workspace
                .open_on_host(&target, "nano".into(), vec![], None, "x".into(), window, cx)
                .unwrap_err();
            assert!(matches!(
                error,
                crate::OpenOnHostError::NoConnectedSession { .. }
            ));
            assert!(opened.borrow().is_empty());
            let online = Remote::new(spec.clone(), true, cx);
            workspace.add_item(online, window, cx);
            let error = workspace
                .open_on_host(
                    &target,
                    "bad\0program".into(),
                    vec![],
                    None,
                    "x".into(),
                    window,
                    cx,
                )
                .unwrap_err();
            assert!(matches!(&error, crate::OpenOnHostError::InvalidLaunch(_)));
            assert!(std::error::Error::source(&error).is_some());
            assert!(opened.borrow().is_empty());
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
