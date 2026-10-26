//! Scoped documents share the ordinary local agent policy and sign-in lifecycle.
use super::*;
use nocterm_ai::{
    AgentConnection, AgentInfo, AgentResourceSettings, AiSettings, SandboxMode,
    history::ContextId,
    model::auth::{AuthMethod, TerminalSetup},
    testing::{ScriptedUsageSource, ScriptedWorkdirResolver},
};

struct LocalLaunch {
    runtime: RuntimeHandle,
    settings: nocterm_store::SettingsHandle,
    directory: tempfile::TempDir,
    connector: Arc<ScriptedConnector>,
    resolver: Arc<ScriptedWorkdirResolver>,
}
struct RunningAgent {
    client: Rc<ProbeClient>,
    _events: async_channel::Sender<nocterm_ai::AgentEvent>,
}
impl RunningAgent {
    fn connection(&self) -> u64 {
        self.client
            .events
            .borrow()
            .iter()
            .find_map(|event| match event {
                SessionEvent::Connected { connection, .. } => Some(*connection),
                _ => None,
            })
            .expect("ordinary connection must be delivered")
    }
}
impl LocalLaunch {
    fn new(mode: SandboxMode, cx: &mut TestAppContext) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connector = Arc::new(ScriptedConnector::default());
        let resolver = Arc::new(ScriptedWorkdirResolver::accept_requests());
        let repository = Arc::new(InMemoryAgentRepository::default());
        let storage =
            AgentStorage::prepared(repository.clone(), repository, Ok(Default::default()));
        let (runtime, settings) = cx.update(|cx| {
            let mut services = runtime_services(
                connector.clone(),
                storage,
                directory.path(),
                Arc::new(ScriptedUsageSource::default()),
                resolver.clone(),
                cx,
            );
            services.terminal_auth = true;
            let settings = services.settings.clone();
            let runtime = RuntimeHandle::new(services, cx);
            settings
                .update::<AiSettings>(cx, move |settings| {
                    settings.sandbox = mode;
                    settings.resources = resources();
                })
                .detach();
            (runtime, settings)
        });
        Self {
            runtime,
            settings,
            directory,
            connector,
            resolver,
        }
    }
    fn start(&self, scope: ContextId, cx: &mut TestAppContext) -> RunningAgent {
        let handle = self.runtime.for_scope(scope);
        let mut metadata = snapshot().metadata.clone();
        metadata.version = 2;
        metadata.header.scope = scope;
        let client = ProbeClient::new(
            cx,
            Arc::new(SharedChat {
                metadata,
                queue: vec![],
            }),
        );
        let (events, receiver) = async_channel::unbounded();
        self.connector.push(Ok(AgentConnection {
            commands: closing::BlockedCommands::new(false),
            events: receiver,
            info: AgentInfo {
                name: "ordinary local agent".into(),
                version: "1".into(),
                capabilities: Default::default(),
                auth_methods: vec![auth_method()],
            },
        }));
        cx.update(|cx| {
            ownership::register_bound(&handle, client.clone(), cx).unwrap();
            handle
                .connect(
                    client.clone(),
                    client.chat.borrow().header.id.clone(),
                    launch(),
                    cx,
                )
                .unwrap();
        });
        cx.run_until_parked();
        let running = RunningAgent {
            client,
            _events: events,
        };
        running.connection();
        running
    }
}
fn resources() -> AgentResourceSettings {
    AgentResourceSettings {
        memory_high_mb: 128,
        memory_max_mb: 256,
        memory_swap_max_mb: 0,
        tasks_max: 32,
    }
}
fn launch() -> AgentLaunch {
    AgentLaunch {
        id: "codex".into(),
        name: "ordinary local agent".into(),
        command: "unused".into(),
        args: vec!["--base".into()],
        env: Default::default(),
        inherit_env: vec![],
    }
}
fn auth_method() -> AuthMethod {
    AuthMethod::terminal(
        "login",
        "Test login",
        TerminalSetup {
            args: vec!["login".into()],
            env: [("TEST_AUTH_METHOD".into(), "1".into())].into(),
        },
    )
}
fn scopes() -> [ContextId; 2] {
    [ContextId::General, contexts::project_chat().header.scope]
}

#[gpui::test]
fn workspace_and_general_chats_offer_the_same_ordinary_terminal_sign_in(cx: &mut TestAppContext) {
    let fixture = LocalLaunch::new(SandboxMode::Off, cx);
    for scope in scopes() {
        let running = fixture.start(scope, cx);
        cx.update(|cx| {
            let request = fixture
                .runtime
                .for_scope(scope)
                .terminal_auth_request(running.connection(), &auth_method(), cx)
                .expect("workspace scope must not suppress the ordinary sign-in command");
            assert_eq!(request.program, "unused");
            assert_eq!(request.args, ["--base", "login"]);
            assert_eq!(
                request.env.get("TEST_AUTH_METHOD").map(String::as_str),
                Some("1")
            );
            assert_eq!(request.cwd, fixture.directory.path().join("work"));
            assert_eq!(running.client.state(cx).unwrap().origin().scope(), scope);
        });
    }
    assert!(
        fixture
            .connector
            .requests()
            .iter()
            .all(|request| request.terminal_auth)
    );
}

#[gpui::test]
fn workspace_and_general_chats_apply_the_same_configured_sandbox_and_resource_limits(
    cx: &mut TestAppContext,
) {
    for mode in [SandboxMode::Off, SandboxMode::Workspace] {
        let fixture = LocalLaunch::new(mode, cx);
        let _running = scopes().map(|scope| fixture.start(scope, cx));
        let preparation = fixture.resolver.requests();
        let requests = fixture.connector.requests();
        assert_eq!(preparation.len(), 2);
        assert_eq!(requests.len(), 2);
        for (prepared, connected) in preparation.iter().zip(&requests) {
            assert_eq!(prepared.isolation.is_some(), mode == SandboxMode::Workspace);
            assert_eq!(connected.sandbox.is_some(), mode == SandboxMode::Workspace);
            assert_eq!(connected.resources, resources());
            assert_eq!(
                connected.working_directory,
                fixture.directory.path().join("work")
            );
        }
        assert_eq!(requests[0].sandbox, requests[1].sandbox);
    }
}

#[gpui::test]
fn changing_the_existing_isolation_setting_retires_workspace_and_general_connections(
    cx: &mut TestAppContext,
) {
    let fixture = LocalLaunch::new(SandboxMode::Off, cx);
    let running = scopes().map(|scope| fixture.start(scope, cx));
    cx.update(|cx| {
        fixture
            .settings
            .update::<AiSettings>(cx, |settings| {
                settings.sandbox = SandboxMode::Workspace;
            })
            .detach();
        fixture.runtime.settings_changed(cx);
    });
    let requests = fixture.connector.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.cancellation.is_cancelled())
    );
    cx.update(|cx| {
        for agent in &running {
            let key = agent.connection();
            assert!(agent.client.events.borrow().iter().any(|event| {
                matches!(event, SessionEvent::Stopped { connection, .. } if *connection == key)
            }));
            let state = agent.client.state(cx).unwrap();
            assert!(
                fixture
                    .runtime
                    .for_scope(state.origin().scope())
                    .document_is_current(state.origin().key().id(), agent.client.id(), cx,)
            );
        }
    });
}
