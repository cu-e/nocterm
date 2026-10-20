use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui_kit::{TestAppContext, WindowOptions, prelude::*};
use nocterm_session::{Auth, HostAddress, RemoteFs, Target};

use crate::{SessionSpec, Workspace};

use crate::test_support::SessionProbe as Remote;

#[gpui_kit::test]
fn programs_open_on_a_connected_host_with_its_sign_in(cx: &mut TestAppContext) {
    let target = Target::parse("root@vm", None).unwrap();
    let host = HostAddress::direct(target.clone());
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
        jumps: Vec::new(),
    };
    let (window, workspace) = workspace_recording(record, cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let offline = Remote::new(spec.clone(), false, cx);
            workspace.add_item(offline, window, cx);
            let error = workspace
                .open_on_host(&host, "nano".into(), vec![], None, "x".into(), window, cx)
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
                    &host,
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
                    &host,
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

type Window = gpui_kit::AnyWindowHandle;

/// A workspace whose opened sessions are pushed to `record`.
fn workspace_recording(
    record: Rc<RefCell<Vec<SessionSpec>>>,
    cx: &mut TestAppContext,
) -> (Window, gpui_kit::Entity<Workspace>) {
    cx.update(|cx| {
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
    })
}

#[gpui_kit::test]
fn programs_open_on_a_connected_host_through_its_jump_hosts(cx: &mut TestAppContext) {
    let target = Target::parse("root@vm", None).unwrap();
    let opened = Rc::new(RefCell::new(Vec::<SessionSpec>::new()));
    let jump = nocterm_session::ResolvedJump {
        host: nocterm_session::JumpHost {
            target: Target::new("ops", "bastion", 22),
            auth: Auth::Password,
        },
        credential: None,
    };
    let host = HostAddress::new(target.clone(), vec![jump.host.target.clone()]);
    let spec = SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: None,
        target: target.clone(),
        auth: Auth::Auto,
        launch: None,
        credential: None,
        jumps: vec![jump.clone()],
    };
    let (window, workspace) = workspace_recording(opened.clone(), cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let online = Remote::new(spec, true, cx);
            workspace.add_item(online, window, cx);
            workspace
                .open_on_host(&host, "nano".into(), vec![], None, "x".into(), window, cx)
                .unwrap();
        })
    })
    .unwrap();
    assert_eq!(opened.borrow()[0].jumps, [jump]);
}

/// Two connected tabs to the same destination behind different bastions.
struct TwoRoutes {
    first: HostAddress,
    second: HostAddress,
    first_fs: Arc<dyn RemoteFs>,
    second_fs: Arc<dyn RemoteFs>,
}

fn bastion(name: &str) -> nocterm_session::ResolvedJump {
    nocterm_session::ResolvedJump {
        host: nocterm_session::JumpHost {
            target: Target::new("ops", name, 22),
            auth: Auth::Auto,
        },
        credential: None,
    }
}

fn spec_via(target: &Target, bastion: &nocterm_session::ResolvedJump) -> SessionSpec {
    SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: None,
        target: target.clone(),
        auth: Auth::Auto,
        launch: None,
        credential: None,
        jumps: vec![bastion.clone()],
    }
}

/// Opens the two tabs, the second one active.
fn two_routes(
    workspace: &gpui_kit::Entity<Workspace>,
    window: Window,
    cx: &mut TestAppContext,
) -> TwoRoutes {
    let target = Target::parse("root@192.168.122.10", None).unwrap();
    let (one, two) = (bastion("hypervisor-a"), bastion("hypervisor-b"));
    let first_fs: Arc<dyn RemoteFs> = Arc::new(crate::workspace::tests::MarkerFs);
    let second_fs: Arc<dyn RemoteFs> = Arc::new(crate::workspace::tests::MarkerFs);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            for (bastion, fs) in [(&one, &first_fs), (&two, &second_fs)] {
                let tab = Remote::with_fs(spec_via(&target, bastion), true, Some(fs.clone()), cx);
                workspace.add_item(tab, window, cx);
            }
        })
    })
    .unwrap();
    TwoRoutes {
        first: HostAddress::new(target.clone(), vec![one.host.target]),
        second: HostAddress::new(target, vec![two.host.target]),
        first_fs,
        second_fs,
    }
}

#[gpui_kit::test]
fn equal_targets_behind_different_jump_hosts_never_share_a_file_system(cx: &mut TestAppContext) {
    let (window, workspace) = workspace_recording(Default::default(), cx);
    let routes = two_routes(&workspace, window, cx);
    cx.update_window(window, |_, _, cx| {
        let workspace = workspace.read(cx);
        // The active tab is the second one; the first route must still win its own lookup.
        let first = workspace.connected_session_for_host(&routes.first).unwrap();
        assert!(Arc::ptr_eq(first.fs.as_ref().unwrap(), &routes.first_fs));
        let second = workspace
            .connected_session_for_host(&routes.second)
            .unwrap();
        assert!(Arc::ptr_eq(second.fs.as_ref().unwrap(), &routes.second_fs));
        let direct = HostAddress::direct(routes.first.target.clone());
        assert!(workspace.connected_session_for_host(&direct).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn files_open_on_the_host_reached_through_the_same_jump_hosts(cx: &mut TestAppContext) {
    let opened = Rc::new(RefCell::new(Vec::<SessionSpec>::new()));
    let (window, workspace) = workspace_recording(opened.clone(), cx);
    let routes = two_routes(&workspace, window, cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace
                .open_on_host(
                    &routes.first,
                    "nano".into(),
                    vec![],
                    None,
                    "x".into(),
                    window,
                    cx,
                )
                .unwrap();
            let direct = HostAddress::direct(routes.first.target.clone());
            let error = workspace
                .open_on_host(&direct, "nano".into(), vec![], None, "x".into(), window, cx)
                .unwrap_err();
            assert!(matches!(
                error,
                crate::OpenOnHostError::NoConnectedSession { .. }
            ));
        })
    })
    .unwrap();
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].address(), routes.first);
}
