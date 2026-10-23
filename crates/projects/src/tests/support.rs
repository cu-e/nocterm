use super::*;

struct NoTerminals;
impl ProjectTerminalOpen for NoTerminals {
    fn capture(
        &self,
        _: Rc<dyn nocterm_workspace::ItemHandle>,
        _: nocterm_workspace::ExpectedSession,
        _: &ProjectTarget,
        _: &App,
    ) -> Result<
        Rc<dyn nocterm_workspace::ProjectTerminalSession>,
        nocterm_workspace::ContextSelectionError,
    > {
        Err(nocterm_workspace::ContextSelectionError::Unavailable)
    }
}

pub(super) fn definition(value: u8) -> ProjectDefinition {
    ProjectDefinition::try_from(ProjectDraft {
        id: ProjectId::from_bytes([value; 16]),
        name: format!("Workspace {value}"),
        target: ProjectTarget::HostFolder {
            host: HostLocator::Local,
            root: RootPath::parse(format!("/srv/project-{value}")).unwrap(),
        },
        source: ProjectSource::Manual,
        updated: 123,
    })
    .unwrap()
}

pub(super) struct Fixture {
    pub window: AnyWindowHandle,
    pub workspace: Entity<Workspace>,
    pub installed: Installed,
    pub model: ProjectsHandle,
}

pub(super) fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, Installed) {
    let rig = Fixture::new(
        vec![definition(1), definition(2)],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        cx,
    );
    (rig.window, rig.workspace, rig.installed)
}

impl Fixture {
    pub(super) fn new(
        projects: Vec<ProjectDefinition>,
        writer: nocterm_store::SnapshotWriter<Catalog>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::with_ports(projects, writer, Rc::new(NoTerminals), None, cx)
    }
    pub(super) fn with_ports(
        projects: Vec<ProjectDefinition>,
        writer: nocterm_store::SnapshotWriter<Catalog>,
        terminals: Rc<dyn ProjectTerminalOpen>,
        resolver: Option<Rc<dyn nocterm_workspace::ProjectTerminalResolve>>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::with_setup(projects, writer, terminals, resolver, None, cx)
    }
    fn with_setup(
        projects: Vec<ProjectDefinition>,
        writer: nocterm_store::SnapshotWriter<Catalog>,
        terminals: Rc<dyn ProjectTerminalOpen>,
        resolver: Option<Rc<dyn nocterm_workspace::ProjectTerminalResolve>>,
        preparation: Option<Arc<dyn nocterm_project_providers::ProjectPreparation>>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::with_installation(
            projects,
            writer,
            |mut installed| {
                if let Some(resolver) = resolver {
                    installed = installed.with_terminal_resolver(resolver);
                }
                if let Some(preparation) = preparation {
                    installed = installed.with_preparation(preparation);
                }
                installed
            },
            terminals,
            cx,
        )
    }
    pub(super) fn with_installation(
        projects: Vec<ProjectDefinition>,
        writer: nocterm_store::SnapshotWriter<Catalog>,
        configure: impl FnOnce(Installed) -> Installed,
        terminals: Rc<dyn ProjectTerminalOpen>,
        cx: &mut TestAppContext,
    ) -> Self {
        let (model, installed) = cx.update(|cx| {
            cx.set_reduce_motion(true);
            gpui_kit::init(cx);
            cx.set_global(Design::new(DesignTokens::builtin()));
            let model = ProjectsHandle::install(
                PreparedProjects::loaded(Ok(Some(Catalog::new(projects).unwrap())), true),
                writer,
                Arc::new(testing::SequenceProjectMetadata::new(10_000, 123)),
                &nocterm_store::ShutdownCoordinator::default(),
                cx,
            )
            .unwrap();
            let installed = configure(init(
                model.clone(),
                terminals,
                nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
                cx,
            ));
            (model, installed)
        });
        let (window, workspace) = open_window(&installed, cx);
        Self {
            window,
            workspace,
            installed,
            model,
        }
    }
    pub(super) fn loaded(projects: Vec<ProjectDefinition>, cx: &mut TestAppContext) -> Self {
        Self::new(projects, Arc::new(|_| Box::pin(async { Ok(()) })), cx)
    }
    pub(super) fn preparing(
        preparation: nocterm_project_providers::testing::FakeProjectPreparation,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::with_setup(
            vec![],
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Rc::new(NoTerminals),
            None,
            Some(Arc::new(preparation)),
            cx,
        )
    }
    pub(super) fn local(
        preparation: nocterm_project_providers::testing::FakeProjectPreparation,
        cx: &mut TestAppContext,
    ) -> (
        Self,
        Arc<dyn nocterm_session::HostExec>,
        Arc<nocterm_session::testing::InMemoryFs>,
    ) {
        let exec: Arc<dyn nocterm_session::HostExec> =
            Arc::new(nocterm_session::testing::FakeExec::default());
        let fs = Arc::new(nocterm_session::testing::InMemoryFs::default());
        fs.seed_file("/home/alice/README", b"home").unwrap();
        fs.set_home("/home/alice").unwrap();
        let rig = Self::with_installation(
            vec![],
            Arc::new(|_| Box::pin(async { Ok(()) })),
            |installed| {
                installed
                    .with_local_host(Some(exec.clone()))
                    .with_preparation(Arc::new(preparation))
                    .with_directory_readers(Arc::new(
                        nocterm_explorer::testing::MemoryDirectoryReaders::new(fs.clone()),
                    ))
            },
            Rc::new(NoTerminals),
            cx,
        );
        (rig, exec, fs)
    }
    pub(super) fn parent(
        &self,
        host: HostLocator,
        title: &str,
        cx: &mut TestAppContext,
    ) -> ParentConnection {
        self.parent_with_fs(host, title, None, cx)
    }
    pub(super) fn parent_with_fs(
        &self,
        host: HostLocator,
        title: &str,
        fs: Option<Arc<dyn nocterm_session::RemoteFs>>,
        cx: &mut TestAppContext,
    ) -> ParentConnection {
        let (target, profile) = match &host {
            HostLocator::Ssh {
                target, profile, ..
            } => (target.clone(), Some(profile.clone().into())),
            HostLocator::SshDirect { target, .. } => (target.clone(), None),
            HostLocator::Local => panic!("SessionProbe represents remote connections"),
        };
        let jumps = match &host {
            HostLocator::Ssh { via, .. } | HostLocator::SshDirect { via, .. } => via
                .iter()
                .cloned()
                .map(|target| nocterm_session::ResolvedJump {
                    host: nocterm_session::JumpHost {
                        target,
                        auth: nocterm_session::Auth::Auto,
                    },
                    credential: None,
                })
                .collect(),
            HostLocator::Local => vec![],
        };
        let port = Rc::new(nocterm_workspace_ports::testing::StaticProjectParent::new(
            nocterm_workspace_ports::ProjectParentDescriptor::new(host, title.to_owned().into())
                .unwrap(),
            Some("/srv/original-folder".into()),
        ));
        let item = cx
            .update_window(self.window, |_, window, cx| {
                self.workspace.update(cx, |workspace, cx| {
                    let item = nocterm_workspace::test_support::SessionProbe::with_fs(
                        nocterm_workspace::SessionSpec {
                            options: Default::default(),
                            title: title.to_owned().into(),
                            profile,
                            target,
                            auth: nocterm_session::Auth::Auto,
                            launch: None,
                            credential: None,
                            jumps,
                        },
                        true,
                        fs,
                        cx,
                    );
                    item.update(cx, |item, cx| item.set_project_parent(port, cx));
                    workspace.add_item(item.clone(), window, cx);
                    item
                })
            })
            .unwrap();
        cx.run_until_parked();
        ParentConnection { item }
    }
}

pub(super) struct ParentConnection {
    pub item: Entity<nocterm_workspace::test_support::SessionProbe>,
}

pub(super) fn open_window(
    installed: &Installed,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let (handle, workspace) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Workspace::new(
                    nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx)),
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        workspace.update(cx, |workspace, cx| {
            register(workspace, installed, window, cx);
            workspace.activate_panel_of::<ProjectsPanel>(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    (handle, workspace)
}

pub(super) fn remote(profile: Option<&str>, host: &str) -> HostLocator {
    let target = nocterm_session::Target::new("alice", host, 22);
    match profile {
        Some(profile) => HostLocator::Ssh {
            profile: profile.into(),
            target,
            via: vec![],
        },
        None => HostLocator::SshDirect {
            target,
            via: vec![],
        },
    }
}

pub(super) fn on_host(value: u8, host: HostLocator) -> ProjectDefinition {
    let mut draft = definition(value).to_draft();
    draft.target = ProjectTarget::HostFolder {
        host,
        root: draft.target.host_folder().unwrap().1.clone(),
    };
    draft.try_into().unwrap()
}

pub(super) fn fill(window: &mut gpui_kit::Window, id: &str, text: &str, cx: &mut App) {
    wizard_field(window, id, cx);
    window.click(id.to_owned(), cx);
    window.press("ctrl-a", cx);
    if text.is_empty() {
        window.press("backspace", cx);
    } else {
        window.input(text, cx);
    }
}

pub(super) fn reveal_form(window: &mut gpui_kit::Window, id: &str, cx: &mut App) {
    wizard_field(window, id, cx);
    window.render_frame(cx);
    let target = window.find(id.to_owned()).bounds();
    let body = window.find("project-form-body").bounds();
    if target.top() < body.top() || target.bottom() > body.bottom() {
        window.scroll(
            "project-form-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                body.center().y - target.center().y,
            )),
            cx,
        );
        window.render_frame(cx);
    }
}

/// Follow the visible wizard actions before using a retained field in older regressions.
pub(super) fn wizard_field(window: &mut gpui_kit::Window, id: &str, cx: &mut App) {
    window.render_frame(cx);
    if window.try_find("project-wizard-steps").is_none() {
        return;
    }
    let project = matches!(id, "project-name" | "project-path" | "project-git");
    let tools = id.starts_with("project-language") || id.starts_with("project-version");
    let connection = id.starts_with("project-mode")
        || id.starts_with("project-image")
        || id.starts_with("project-engine")
        || id == "project-custom-image"
        || id == "project-registry-images";
    let heading = if project {
        "Project"
    } else if tools {
        "Tools"
    } else if connection {
        "Connection"
    } else {
        return;
    };
    for _ in 0..3 {
        let current = window
            .find("project-step-heading")
            .label()
            .unwrap()
            .to_owned();
        if current == heading {
            return;
        }
        if current == "Tools" || heading == "Connection" {
            window.click("project-back", cx);
        } else {
            window.click("project-next", cx);
        }
        window.render_frame(cx);
    }
}
pub(super) fn create_workspace(window: &mut gpui_kit::Window, cx: &mut App) {
    window.render_frame(cx);
    for _ in 0..3 {
        if window.try_find("project-save").is_some() {
            window.click("project-save", cx);
            return;
        }
        window.click("project-next", cx);
        window.render_frame(cx);
        if window.try_find("project-form-error").is_some() {
            return;
        }
    }
}
