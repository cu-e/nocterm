//! Original destinations are checked before any native session is started.
use super::*;
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext, WindowOptions};
use nocterm_project_model::{ProjectId, RootPath};
use nocterm_session::{
    ExecRequest,
    testing::{FakeExec, ScriptedTransport},
};
use nocterm_workspace::{ProjectSurface, test_support::ProjectSurfaceProbe};
use nocterm_workspace_ports::ContextSelectionError;
use std::{rc::Rc, sync::Arc};

struct Fixture {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    installed: Installed,
    transport: Arc<ScriptedTransport>,
}
fn fixture(cx: &mut TestAppContext) -> Fixture {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let transport = Arc::new(ScriptedTransport::default());
        let local: Arc<dyn nocterm_session::Transport> = transport.clone();
        let installed = install(
            Deps::new(
                transport.clone(),
                LocalSessions::new(Arc::new(move |_| local.clone()), "/bin/sh".into()),
            ),
            &ready,
            cx,
        );
        let layout = nocterm_ui::LayoutHandle::in_memory(cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(nocterm_workspace::Deps::new(layout), window, cx))
            })
            .unwrap();
        Fixture {
            window,
            workspace,
            installed,
            transport,
        }
    })
}
fn spec() -> SessionSpec {
    SessionSpec {
        options: Default::default(),
        title: "Server".into(),
        profile: None,
        target: nocterm_session::Target::new("user", "server", 22),
        auth: nocterm_session::Auth::Auto,
        launch: None,
        credential: None,
        jumps: vec![],
    }
}
fn project(f: &Fixture, id: u8, window: &mut Window, cx: &mut App) -> ProjectSurface {
    let factory = f.workspace.read(cx).project_surface_factory();
    let view = cx.new(|cx| {
        ProjectSurfaceProbe::new(factory, ProjectId::from_bytes([id; 16]), window, cx).unwrap()
    });
    let surface = view.read(cx).surface();
    f.workspace.update(cx, |workspace, cx| {
        workspace
            .add_project_workspace(view, surface.clone(), window, cx)
            .unwrap()
    });
    surface
}
#[gpui_kit::test]
fn every_general_opener_refuses_another_native_window_before_starting_a_process(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let other = fixture(cx);
    let exec = Arc::new(FakeExec::default());
    cx.update_window(other.window, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            assert!(matches!(
                open_session(&f.installed, workspace, spec(), window, cx),
                Err(ContextSelectionError::StaleWindow)
            ));
            assert!(matches!(
                open_local(
                    &f.installed,
                    workspace,
                    nocterm_workspace::LocalTerminalTarget::Bottom,
                    window,
                    cx
                ),
                Err(ContextSelectionError::StaleWindow)
            ));
            assert!(matches!(
                open_background_session(&f.installed, workspace, spec(), window, cx),
                Err(ContextSelectionError::StaleWindow)
            ));
            let command = LocalCommand::new(
                Default::default(),
                "Command".into(),
                f.transport.clone(),
                |_, _| {},
            );
            assert!(matches!(
                open_local_command(&f.installed, workspace, command, window, cx),
                Err(ContextSelectionError::StaleWindow)
            ));
            let executor: Arc<dyn nocterm_session::HostExec> = exec.clone();
            let program = ProgramSpec {
                title: "Program".into(),
                host: nocterm_workspace::Host::resolve(None, Some(&executor)).unwrap(),
                program: ExecRequest::new("/bin/sh"),
                shell_syntax: None,
            };
            assert!(matches!(
                open_program(&f.installed, workspace, program, window, cx),
                Err(ContextSelectionError::StaleWindow)
            ));
            assert_eq!(workspace.items(cx).count(), 0);
        })
    })
    .unwrap();
    assert!(f.transport.requests().is_empty());
    assert!(exec.requests().is_empty());
}

fn connected_parent(
    f: &Fixture,
    spec: SessionSpec,
    cx: &mut TestAppContext,
) -> (
    Rc<dyn nocterm_workspace::ItemHandle>,
    nocterm_session::SessionDriver,
) {
    let driver = f.transport.queue(None, None);
    cx.update_window(f.window, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            open_session(&f.installed, workspace, spec, window, cx).unwrap()
        })
    })
    .unwrap();
    assert!(
        futures::FutureExt::now_or_never(driver.emit(nocterm_session::Event::Connected)).unwrap()
    );
    cx.run_until_parked();
    let parent = cx.update(|cx| f.workspace.read(cx).active_item(cx).unwrap());
    (parent, driver)
}
fn capture_parent(
    f: &Fixture,
    parent: Rc<dyn nocterm_workspace::ItemHandle>,
    root: &RootPath,
    cx: &mut TestAppContext,
) -> Rc<dyn nocterm_workspace_ports::ProjectTerminalSession> {
    cx.update(|cx| {
        let expected = parent
            .terminal_capabilities(cx)
            .unwrap()
            .state()
            .info(cx)
            .unwrap()
            .expected;
        let source = parent
            .terminal_capabilities(cx)
            .unwrap()
            .project_parent()
            .unwrap()
            .capture(expected, cx)
            .unwrap();
        let location = nocterm_project_model::ProjectTarget::HostFolder {
            host: source.parent().host().clone(),
            root: root.clone(),
        };
        project_terminal_opener(&f.installed)
            .capture(
                parent.view(),
                expected,
                parent.terminal_capabilities(cx).unwrap(),
                &location,
                cx,
            )
            .unwrap()
    })
}
fn open_hidden_and_close_parent(
    f: &Fixture,
    session: &Rc<dyn nocterm_workspace_ports::ProjectTerminalSession>,
    parent: Rc<dyn nocterm_workspace::ItemHandle>,
    cx: &mut TestAppContext,
) -> ProjectSurface {
    let surface = cx
        .update_window(f.window, |_, window, cx| {
            let a = project(f, 1, window, cx);
            let b = project(f, 2, window, cx);
            session
                .open(a.initial_terminal_target(window, cx).unwrap(), window, cx)
                .unwrap();
            assert_eq!(
                f.workspace.read(cx).selected_context(cx),
                nocterm_project_model::ContextId::Project(b.project())
            );
            f.workspace.update(cx, |workspace, cx| {
                workspace.close_tabs(
                    parent.item_id(),
                    nocterm_workspace::TabCloseScope::Current,
                    window,
                    cx,
                )
            });
            a
        })
        .unwrap();
    cx.run_until_parked();
    surface
}
#[gpui_kit::test]
fn folder_terminals_keep_literal_cwd_and_normal_reconnect_after_parent_closes(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let mut original = spec();
    original.launch = Some(nocterm_session::ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec!["--noprofile".into()],
        env: [("PROJECT_TEST".into(), "literal".into())].into(),
        ..Default::default()
    });
    let (parent, _driver) = connected_parent(&f, original.clone(), cx);
    let root = RootPath::parse("/root/space ' directory").unwrap();
    let session = capture_parent(&f, parent.clone(), &root, cx);
    f.transport.queue(None, None);
    let surface = open_hidden_and_close_parent(&f, &session, parent, cx);
    let requests = f.transport.requests();
    assert_eq!(requests.len(), 2);
    let launch = &requests[1].launch;
    assert_eq!(launch.cwd.as_deref(), Some(root.as_str()));
    assert_eq!(launch.program, original.launch.as_ref().unwrap().program);
    assert_eq!(launch.args, original.launch.as_ref().unwrap().args);
    assert_eq!(launch.env, original.launch.as_ref().unwrap().env);
    f.transport.queue(None, None);
    cx.update_window(f.window, |_, _, cx| {
        let view = surface.find_item::<TerminalView>(cx).unwrap();
        let capabilities = nocterm_workspace::ItemHandle::terminal_capabilities(&view, cx).unwrap();
        let expected = capabilities.state().info(cx).unwrap().expected;
        assert!(capabilities.prompts().is_some());
        capabilities
            .reconnect()
            .unwrap()
            .reconnect(expected, cx)
            .unwrap();
    })
    .unwrap();
    assert_eq!(f.transport.requests().len(), 3);
    assert_eq!(
        f.transport.requests()[2].launch.cwd.as_deref(),
        Some(root.as_str())
    );
    f.transport.queue(None, None);
    cx.update_window(f.window, |_, window, cx| {
        let outer = surface.source(cx).outer_item().unwrap();
        f.workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        let target = surface
            .terminal_target(surface.selection_receipt(cx), None, window, cx)
            .unwrap();
        session.open(target, window, cx).unwrap();
    })
    .unwrap();
    assert_eq!(f.transport.requests().len(), 4);
    assert_eq!(
        f.transport.requests()[3].launch.cwd.as_deref(),
        Some(root.as_str())
    );
}

#[gpui_kit::test]
fn source_capture_refuses_a_replacement_epoch_before_opening_a_folder_terminal(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (parent, _driver) = connected_parent(&f, spec(), cx);
    cx.update_window(f.window, |_, _, cx| {
        let capabilities = parent.terminal_capabilities(cx).unwrap();
        let expected = capabilities.state().info(cx).unwrap().expected;
        f.transport.queue(None, None);
        let view = parent.view().downcast::<TerminalView>().unwrap();
        let terminal = view.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| terminal.reconnect(cx));
        assert!(
            project_terminal_opener(&f.installed)
                .capture(
                    parent.view(),
                    expected,
                    capabilities,
                    &nocterm_project_model::ProjectTarget::HostFolder {
                        host: nocterm_project_model::HostLocator::SshDirect {
                            target: spec().target,
                            via: vec![]
                        },
                        root: RootPath::parse("/root").unwrap()
                    },
                    cx
                )
                .is_err()
        );
    })
    .unwrap();
    assert_eq!(f.transport.requests().len(), 2);
}

#[gpui_kit::test]
fn folder_sessions_preserve_original_ordered_jump_hosts_and_credentials(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut original = spec();
    original.profile = Some("saved-server".into());
    original.credential = Some(nocterm_session::CredentialId::from_bytes([3; 16]));
    original.jumps = ["first", "second"]
        .into_iter()
        .enumerate()
        .map(|(index, host)| nocterm_session::ResolvedJump {
            host: nocterm_session::JumpHost {
                target: nocterm_session::Target::new("hop", host, 2222),
                auth: nocterm_session::Auth::Password,
            },
            credential: Some(nocterm_session::CredentialId::from_bytes(
                [index as u8 + 1; 16],
            )),
        })
        .collect();
    let (parent, _driver) = connected_parent(&f, original.clone(), cx);
    let session = capture_parent(&f, parent, &RootPath::parse("/root").unwrap(), cx);
    f.transport.queue(None, None);
    cx.update_window(f.window, |_, window, cx| {
        let surface = project(&f, 1, window, cx);
        session
            .open(
                surface.initial_terminal_target(window, cx).unwrap(),
                window,
                cx,
            )
            .unwrap();
        let terminal = surface
            .find_item::<TerminalView>(cx)
            .unwrap()
            .read(cx)
            .terminal()
            .clone();
        let captured = terminal.read(cx).spec();
        assert_eq!(captured.profile, original.profile);
        assert_eq!(captured.credential, original.credential);
        assert_eq!(captured.jumps, original.jumps);
    })
    .unwrap();
    let requests = f.transport.requests();
    assert_eq!(requests[1].jumps, requests[0].jumps);
    assert_eq!(requests[1].target, requests[0].target);
    assert_eq!(requests[1].launch.cwd.as_deref(), Some("/root"));
}

#[gpui_kit::test]
fn local_folder_explorer_uses_its_saved_root_without_ssh_session_metadata(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let driver = f.transport.queue(None, None);
    let id = cx
        .update_window(f.window, |_, window, cx| {
            f.workspace.update(cx, |workspace, cx| {
                open_local(
                    &f.installed,
                    workspace,
                    nocterm_workspace::LocalTerminalTarget::Bottom,
                    window,
                    cx,
                )
                .unwrap()
            })
        })
        .unwrap();
    assert!(
        futures::FutureExt::now_or_never(driver.emit(nocterm_session::Event::Connected)).unwrap()
    );
    cx.run_until_parked();
    let parent = cx.update(|cx| f.workspace.read(cx).general_item(id, cx).unwrap());
    let session = capture_parent(&f, parent, &RootPath::parse("/root").unwrap(), cx);
    f.transport.queue(None, None);
    let root = RootPath::parse("/root").unwrap();
    cx.update_window(f.window, |_, window, cx| {
        let surface = project(&f, 1, window, cx);
        surface.set_root(root.clone(), cx);
        session
            .open(
                surface.initial_terminal_target(window, cx).unwrap(),
                window,
                cx,
            )
            .unwrap();
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let source = f.workspace.read(cx).explorer_source(cx).unwrap();
        assert!(source.is_local());
        assert_eq!(source.initial_directory(), Some("/root"));
        assert!(source.filesystem().is_none());
    });
    assert_eq!(
        f.transport.requests()[1].launch.cwd.as_deref(),
        Some("/root")
    );
}

#[gpui_kit::test]
fn an_offline_folder_configuration_authenticates_in_its_inner_terminal(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let original = spec();
    let session = project_session(
        &f.installed,
        original.clone(),
        false,
        &nocterm_project_model::ProjectTarget::HostFolder {
            host: nocterm_project_model::HostLocator::SshDirect {
                target: original.target.clone(),
                via: vec![],
            },
            root: RootPath::parse("/srv/project").unwrap(),
        },
    )
    .unwrap();
    assert!(f.transport.requests().is_empty());
    let driver = f.transport.queue(None, None);
    let surface = cx
        .update_window(f.window, |_, window, cx| {
            let surface = project(&f, 1, window, cx);
            session
                .open(
                    surface.initial_terminal_target(window, cx).unwrap(),
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(f.workspace.read(cx).items(cx).count(), 1);
            surface
        })
        .unwrap();
    let (reply, answer) = nocterm_session::Reply::channel();
    assert!(
        futures::FutureExt::now_or_never(driver.emit(nocterm_session::Event::Prompt(
            nocterm_session::Prompt::Secret {
                request: nocterm_session::SecretRequest::Password {
                    target: original.target,
                    retry: false
                },
                hop: None,
                reply,
            }
        )))
        .unwrap()
    );
    cx.run_until_parked();
    cx.update(|cx| {
        let view = surface.find_item::<TerminalView>(cx).unwrap();
        let capabilities = nocterm_workspace::ItemHandle::terminal_capabilities(&view, cx).unwrap();
        assert_eq!(
            capabilities.state().info(cx).unwrap().status,
            nocterm_workspace::TerminalStatus::AwaitingUser
        );
        let prompts = capabilities.prompts().unwrap();
        let prompt = prompts.pending(cx).unwrap();
        prompts
            .answer(
                prompt.token,
                nocterm_session::Secret::new("inside workspace"),
                cx,
            )
            .unwrap();
        assert_eq!(f.workspace.read(cx).items(cx).count(), 1);
    });
    assert_eq!(
        futures::executor::block_on(answer)
            .unwrap()
            .unwrap()
            .expose(),
        "inside workspace"
    );
}

#[gpui_kit::test]
fn project_capture_refuses_capabilities_from_another_parent_view(cx: &mut TestAppContext) {
    use nocterm_workspace_ports::ProjectParentError;
    let f = fixture(cx);
    let (first, _first_driver) = connected_parent(&f, spec(), cx);
    let (second, _second_driver) = connected_parent(&f, spec(), cx);
    cx.update(|cx| {
        let capabilities = first.terminal_capabilities(cx).unwrap();
        let expected = capabilities.state().info(cx).unwrap().expected;
        let location = nocterm_project_model::ProjectTarget::HostFolder {
            host: nocterm_project_model::HostLocator::SshDirect {
                target: spec().target,
                via: vec![],
            },
            root: RootPath::parse("/root").unwrap(),
        };
        assert!(matches!(
            project_terminal_opener(&f.installed).capture(
                second.view(),
                expected,
                capabilities,
                &location,
                cx
            ),
            Err(ContextSelectionError::Parent(
                ProjectParentError::StaleSession
            ))
        ));
    });
    assert_eq!(f.transport.requests().len(), 2);
}

#[gpui_kit::test]
fn project_provider_refuses_foreign_destination_before_starting_a_terminal(
    cx: &mut TestAppContext,
) {
    use nocterm_workspace_ports::ProjectTerminalTarget;
    let f = fixture(cx);
    let (parent, _driver) = connected_parent(&f, spec(), cx);
    let session = capture_parent(&f, parent, &RootPath::parse("/root").unwrap(), cx);
    cx.update_window(f.window, |_, window, cx| {
        let target = ProjectTerminalTarget::new(
            cx.new(|_| nocterm_workspace::test_support::ProjectTerminalProbe::default()),
        );
        assert!(target.validate(window, cx).is_ok());
        assert!(matches!(
            session.open(target, window, cx),
            Err(ContextSelectionError::StaleSurface)
        ));
        assert_eq!(f.workspace.read(cx).items(cx).count(), 1);
    })
    .unwrap();
    assert_eq!(f.transport.requests().len(), 1);
}
