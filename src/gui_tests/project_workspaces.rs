//! Workspaces capture real installed terminal capabilities at the app boundary.
use super::*;
use gpui_kit::App;
use nocterm_project_model::{
    Catalog, ContextId, HostLocator, ProjectDefinition, ProjectDraft, ProjectId, ProjectSource,
    RootPath, testing::SequenceProjectMetadata,
};
use nocterm_project_service::{PreparedProjects, ProjectsHandle};
use nocterm_session::testing::{FakeCredentialStore, InMemoryFs, ScriptedTransport};
use nocterm_workspace::LocalTerminalTarget;

mod reconnection;

struct ProjectFixture {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    projects: nocterm_projects::Installed,
    terminal: nocterm_terminal::Installed,
    connections: nocterm_connections::Installed,
    transport: Arc<ScriptedTransport>,
    credentials: Arc<FakeCredentialStore>,
}

impl ProjectFixture {
    fn new(host: HostLocator, cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            gpui_kit::init(cx);
            let ready = nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
                cx,
            );
            let transport = Arc::new(ScriptedTransport::default());
            let credentials = Arc::new(FakeCredentialStore::default());
            let local: Arc<dyn Transport> = transport.clone();
            let terminal = nocterm_terminal::install(
                nocterm_terminal::Deps::new(
                    transport.clone(),
                    nocterm_terminal::LocalSessions::new(
                        Arc::new(move |_| local.clone()),
                        "/bin/sh".into(),
                    ),
                )
                .with_credentials(nocterm_terminal::CredentialProvider::new(
                    credentials.clone(),
                    |_, _, _| gpui_kit::Task::ready(Ok(())),
                )),
                &ready,
                cx,
            );
            let connections = nocterm_connections::install(
                nocterm_connections::PreparedConnections::memory(
                    nocterm_connections_model::draft::EditorDefaults {
                        user: Some("test".into()),
                        home: None,
                    },
                ),
                &ready,
                &nocterm_store::ShutdownCoordinator::default(),
                cx,
            )
            .unwrap();
            Self::compose(host, transport, terminal, connections, credentials, cx)
        })
    }

    fn compose(
        host: HostLocator,
        transport: Arc<ScriptedTransport>,
        terminal: nocterm_terminal::Installed,
        connections: nocterm_connections::Installed,
        credentials: Arc<FakeCredentialStore>,
        cx: &mut App,
    ) -> Self {
        let model = ProjectsHandle::install(
            PreparedProjects::loaded(
                Ok(Some(Catalog::new(vec![definition(host)]).unwrap())),
                true,
            ),
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(SequenceProjectMetadata::new(100, 123)),
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let projects = nocterm_projects::init(
            model.clone(),
            nocterm_terminal::project_terminal_opener(&terminal),
        )
        .with_terminal_resolver(crate::project_setup::terminal_resolver(
            &terminal,
            Some(&connections),
        ));
        let (window, workspace) = project_window(&terminal, &connections, &projects, cx);
        Self {
            window,
            workspace,
            projects,
            terminal,
            connections,
            transport,
            credentials,
        }
    }

    fn show_project(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| {
            self.workspace.update(cx, |workspace, cx| {
                self.projects
                    .open(ProjectId::from_bytes([1; 16]), workspace, window, cx)
                    .unwrap();
            });
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn open_project(&self, cx: &mut TestAppContext) -> SessionDriver {
        let driver = self
            .transport
            .queue(Some(Arc::new(InMemoryFs::default())), None);
        self.show_project(cx);
        connected(&driver, cx);
        driver
    }

    fn local_parent(&self, cx: &mut TestAppContext) -> (gpui_kit::EntityId, SessionDriver) {
        let driver = self.transport.queue(None, None);
        let parent = cx
            .update_window(self.window, |_, window, cx| {
                self.workspace.update(cx, |workspace, cx| {
                    nocterm_terminal::open_local(
                        &self.terminal,
                        workspace,
                        LocalTerminalTarget::Bottom,
                        window,
                        cx,
                    )
                    .unwrap();
                    workspace.selected_local_terminal(cx).unwrap()
                })
            })
            .unwrap();
        connected(&driver, cx);
        (parent, driver)
    }

    fn remote_parent(
        &self,
        spec: SessionSpec,
        cx: &mut TestAppContext,
    ) -> (gpui_kit::EntityId, SessionDriver) {
        let driver = self.transport.queue(None, None);
        let parent = cx
            .update_window(self.window, |_, window, cx| {
                self.workspace.update(cx, |workspace, cx| {
                    nocterm_terminal::open_session(&self.terminal, workspace, spec, window, cx)
                        .unwrap();
                    workspace.active_item(cx).unwrap().item_id()
                })
            })
            .unwrap();
        connected(&driver, cx);
        (parent, driver)
    }

    fn save_server(
        &self,
        profile: nocterm_connections_model::profiles::Profile,
        cx: &mut TestAppContext,
    ) {
        let saved = cx.update(|cx| {
            self.connections
                .connections()
                .update(cx, |connections, cx| connections.save_profile(profile, cx))
        });
        saved.now_or_never().unwrap().unwrap();
    }

    fn assert_first_terminal(&self, parent: gpui_kit::EntityId, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            let workspace = self.workspace.read(cx);
            assert_eq!(
                workspace.selected_context(cx),
                ContextId::Project(ProjectId::from_bytes([1; 16]))
            );
            assert!(window.try_find("project-unavailable").is_none());
            assert_eq!(workspace.terminals(cx).len(), 1);
            assert_eq!(
                workspace.terminals(cx)[0]
                    .capabilities
                    .state()
                    .info(cx)
                    .unwrap()
                    .status,
                nocterm_workspace::TerminalStatus::Connected
            );
            assert!(workspace.general_item(parent, cx).is_some());
            let requests = self.transport.requests();
            let request = requests.last().unwrap();
            assert_eq!(request.launch.cwd.as_deref(), Some("/home/test/project"));
        })
        .unwrap();
    }
}

fn project_window(
    terminal: &nocterm_terminal::Installed,
    connections: &nocterm_connections::Installed,
    projects: &nocterm_projects::Installed,
    cx: &mut App,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let layout = nocterm_ui::LayoutHandle::in_memory(cx);
    gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        window.activate_window();
        cx.new(|cx| {
            let mut workspace = Workspace::new(nocterm_workspace::Deps::new(layout), window, cx);
            nocterm_terminal::register(terminal, &mut workspace);
            nocterm_connections::register(connections, &mut workspace, window, cx);
            workspace.set_chat_connection_opener(crate::project_setup::chat_connection_opener(
                terminal,
                connections,
            ));
            nocterm_projects::register(&mut workspace, projects, window, cx);
            workspace
        })
    })
    .unwrap()
}

fn definition(host: HostLocator) -> ProjectDefinition {
    ProjectDraft {
        id: ProjectId::from_bytes([1; 16]),
        name: "Test workspace".into(),
        host,
        root: RootPath::parse("/home/test/project").unwrap(),
        source: ProjectSource::Manual,
        updated: 123,
    }
    .try_into()
    .unwrap()
}

fn connected(driver: &SessionDriver, cx: &mut TestAppContext) {
    assert!(driver.emit(Event::Connected).now_or_never().unwrap());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_workspace_copies_the_focused_matching_servers_shell_configuration(cx: &mut TestAppContext) {
    let first = spec("same-server.example");
    let f = ProjectFixture::new(
        HostLocator::SshDirect {
            target: first.target.clone(),
            via: vec![],
        },
        cx,
    );
    let mut first = first;
    first.launch = Some(nocterm_session::ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec!["--norc".into()],
        env: [("SOURCE_SESSION".into(), "first".into())].into(),
        cwd: Some("/first".into()),
        integration: false,
    });
    let mut selected = first.clone();
    selected
        .launch
        .as_mut()
        .unwrap()
        .env
        .insert("SOURCE_SESSION".into(), "selected".into());
    f.remote_parent(first, cx);
    f.remote_parent(selected.clone(), cx);
    let _inner = f.open_project(cx);
    let mut expected = selected.launch.unwrap();
    expected.cwd = Some("/home/test/project".into());
    assert_eq!(f.transport.requests().last().unwrap().launch, expected);
}

#[gpui_kit::test]
fn opening_a_workspace_finds_its_real_local_parent_in_the_bottom_dock(cx: &mut TestAppContext) {
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let (parent, driver) = f.local_parent(cx);
    let _terminal = f.open_project(cx);
    f.assert_first_terminal(parent, cx);
    assert!(driver.closed().now_or_never().is_none());
}

#[gpui_kit::test]
fn opening_a_workspace_finds_its_real_local_parent_after_dragging_it_to_the_center(
    cx: &mut TestAppContext,
) {
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let (remote, _remote_driver) = f.remote_parent(spec("unrelated.example"), cx);
    let (local, _local_driver) = f.local_parent(cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            ("tab-title", local.as_u64()),
            ("tab-title", remote.as_u64()),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, _, cx| {
        assert_eq!(
            f.workspace.read(cx).tab_location(local, cx).unwrap().0,
            gpui_kit::component::dock::DockPlacement::Center
        );
    })
    .unwrap();
    let _terminal = f.open_project(cx);
    f.assert_first_terminal(local, cx);
}

#[gpui_kit::test]
fn creating_a_workspace_from_the_focused_local_terminal_keeps_the_local_host(
    cx: &mut TestAppContext,
) {
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let (_remote, _remote_driver) = f.remote_parent(spec("unrelated.example"), cx);
    let (local, _local_driver) = f.local_parent(cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            f.workspace
                .read(cx)
                .command_item(window, cx)
                .unwrap()
                .item_id(),
            local
        );
        window.dispatch_action(Box::new(nocterm_projects::NewProject), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-name", cx);
        window.input("From focused local", cx);
        window.click("project-path", cx);
        window.input("/home/test/from-local", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let catalog = f.projects.catalog(cx);
        let saved = catalog
            .iter()
            .find(|definition| definition.name() == "From focused local")
            .expect("workspace creation should be available for the focused local terminal");
        assert_eq!(saved.host(), &HostLocator::Local);
        assert_eq!(saved.root().as_str(), "/home/test/from-local");
    });
}

fn routed_workspace(hops: usize, cx: &mut TestAppContext) {
    let mut spec = spec("destination.example");
    spec.profile = Some("saved-server".into());
    spec.launch = Some(nocterm_session::ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec!["--noprofile".into(), "--norc".into()],
        env: [("FOLDER_TEST".into(), "literal ; $(false)".into())].into(),
        cwd: Some("/original-directory".into()),
        integration: false,
    });
    spec.options.proxy = Some(nocterm_session::ProxyConfig::HttpConnect {
        host: "proxy.example".into(),
        port: 8080,
    });
    spec.jumps = (0..hops)
        .map(|hop| nocterm_session::ResolvedJump {
            host: nocterm_session::JumpHost {
                target: nocterm_session::Target::new("jump", format!("hop-{hop}.example"), 2222),
                auth: nocterm_session::Auth::Auto,
            },
            credential: None,
        })
        .collect();
    let host = HostLocator::Ssh {
        profile: "saved-server".into(),
        target: spec.target.clone(),
        via: spec.address().via,
    };
    let f = ProjectFixture::new(host.clone(), cx);
    let files = Arc::new(InMemoryFs::default());
    let driver = f.transport.queue(Some(files.clone()), None);
    let parent = cx
        .update_window(f.window, |_, window, cx| {
            f.workspace.update(cx, |workspace, cx| {
                nocterm_terminal::open_session(&f.terminal, workspace, spec, window, cx).unwrap();
                workspace.active_item(cx).unwrap().item_id()
            })
        })
        .unwrap();
    connected(&driver, cx);
    let _terminal = f.open_project(cx);
    f.assert_first_terminal(parent, cx);
    let requests = f.transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].target, requests[0].target);
    assert_eq!(requests[1].jumps, requests[0].jumps);
    assert_eq!(requests[1].auth, requests[0].auth);
    assert_eq!(requests[1].proxy, requests[0].proxy);
    assert_eq!(requests[1].jumps.len(), hops);
    assert_eq!(
        requests[1].launch.cwd.as_deref(),
        Some("/home/test/project")
    );
    let mut expected_launch = requests[0].launch.clone();
    expected_launch.cwd = Some("/home/test/project".into());
    assert_eq!(requests[1].launch, expected_launch);
    cx.update(|cx| {
        let terminals = f.workspace.read(cx).terminals(cx);
        let info = terminals[0].capabilities.state().info(cx).unwrap();
        assert_eq!(info.profile.as_deref(), Some("saved-server"));
        assert!(!info.local);
    });
}

#[gpui_kit::test]
fn opening_a_workspace_captures_the_real_terminal_with_one_jump_host(cx: &mut TestAppContext) {
    routed_workspace(1, cx);
}

#[gpui_kit::test]
fn opening_a_workspace_captures_the_real_terminal_with_two_jump_hosts(cx: &mut TestAppContext) {
    routed_workspace(2, cx);
}

#[gpui_kit::test]
fn new_folder_tabs_keep_their_route_after_the_original_server_tab_closes(cx: &mut TestAppContext) {
    let parent_spec = spec("destination.example");
    let f = ProjectFixture::new(
        HostLocator::SshDirect {
            target: parent_spec.target.clone(),
            via: Vec::new(),
        },
        cx,
    );
    let (_parent, parent_driver) = f.remote_parent(parent_spec, cx);
    let folder = f.open_project(cx);
    cx.update_window(f.window, |_, window, cx| {
        f.workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(parent_driver.closed().now_or_never().is_some());
    assert!(folder.closed().now_or_never().is_none());
    let next = f.transport.queue(None, None);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    connected(&next, cx);
    let requests = f.transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].target, requests[1].target);
    assert_eq!(
        requests[2].launch.cwd.as_deref(),
        Some("/home/test/project")
    );
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 2));
}

#[gpui_kit::test]
fn reconnecting_a_folder_terminal_keeps_its_starting_directory(cx: &mut TestAppContext) {
    let parent_spec = spec("destination.example");
    let f = ProjectFixture::new(
        HostLocator::SshDirect {
            target: parent_spec.target.clone(),
            via: Vec::new(),
        },
        cx,
    );
    let (_parent, _parent_driver) = f.remote_parent(parent_spec, cx);
    let folder = f.open_project(cx);
    assert!(
        folder
            .emit(Event::Closed(nocterm_session::CloseReason::Exited(Some(0))))
            .now_or_never()
            .unwrap()
    );
    cx.run_until_parked();
    let next = f.transport.queue(None, None);
    cx.update(|cx| {
        let entry = f.workspace.read(cx).terminals(cx).remove(0);
        let expected = entry.capabilities.state().info(cx).unwrap().expected;
        let reopened = entry
            .capabilities
            .reconnect()
            .unwrap()
            .reconnect(expected, cx)
            .unwrap();
        assert_eq!(reopened.item(), expected.item());
        assert_ne!(reopened.epoch(), expected.epoch());
    });
    connected(&next, cx);
    let requests = f.transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].launch, requests[1].launch);
    assert_eq!(requests[2].target, requests[1].target);
}
