//! Scripted agent commands, connector and tool bridge behind the panel fixture.
use super::*;

#[derive(Default)]
pub(super) struct Commands {
    pub(super) sessions: AtomicU64,
    pub(super) new_dirs: Mutex<Vec<std::path::PathBuf>>,
    pub(super) restore_dirs: Mutex<Vec<std::path::PathBuf>>,
    pub(super) session_gate: Mutex<Option<oneshot::Receiver<()>>>,
    pub(super) close_gate: Mutex<Option<oneshot::Receiver<()>>>,
    pub(super) closed: Mutex<Vec<SessionId>>,
    pub(super) shutdown_failure: AtomicBool,
    pub(super) auth_required: AtomicBool,
    pub(super) authentications: AtomicUsize,
    pub(super) authentication_failure: AtomicBool,
    pub(super) shutdowns: AtomicUsize,
    pub(super) cancels: AtomicUsize,
    pub(super) prompts: Mutex<Vec<PromptRequest>>,
    pub(super) pending: Mutex<Option<oneshot::Sender<PromptOutcome>>>,
    /// Each reopened session, and whether it was forked.
    pub(super) restores: Mutex<Vec<(SessionId, bool)>>,
    pub(super) restore_errors: Mutex<std::collections::VecDeque<AgentError>>,
    /// The MCP servers each new session was given.
    pub(super) servers: Mutex<Vec<Vec<ExternalToolEndpoint>>>,
    /// The configuration options every session reports.
    pub(super) config: super::config_responses::ConfigResponses,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.new_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.servers.lock().unwrap().push(request.endpoints);
        let gate = self.session_gate.lock().unwrap().take();
        let id = self.sessions.fetch_add(1, Ordering::SeqCst);
        let auth_required = self.auth_required.load(Ordering::SeqCst);
        let config = self.config.options();
        let modes = self.config.modes();
        async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            if auth_required {
                Err(AgentError::AuthRequired("Sign in to continue".into()))
            } else {
                Ok(opened(format!("s{id}"), config, modes))
            }
        }
        .boxed()
    }
    fn restore_session(
        &self,
        request: nocterm_ai::model::session::RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.restore_dirs
            .lock()
            .unwrap()
            .push(request.working_directory.clone());
        self.restores
            .lock()
            .unwrap()
            .push((request.session.clone(), request.fork));
        if let Some(error) = self.restore_errors.lock().unwrap().pop_front() {
            return async move { Err(error) }.boxed();
        }
        let session = if request.fork {
            SessionId::new(format!("fork-of-{}", request.session))
        } else {
            request.session
        };
        let config = self.config.options();
        let modes = self.config.modes();
        async move { Ok(opened(session, config, modes)) }.boxed()
    }
    fn prompt(&self, r: PromptRequest) -> BoxFuture<'static, Result<PromptOutcome, AgentError>> {
        self.prompts.lock().unwrap().push(r);
        if self.auth_required.load(Ordering::SeqCst) {
            return async { Err(AgentError::AuthRequired("Sign in to continue".into())) }.boxed();
        }
        let (tx, rx) = oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        async move { rx.await.map_err(|_| AgentError::Io("closed".into())) }.boxed()
    }
    fn cancel(&self, _: SessionId) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
    }
    fn set_mode(&self, _: SetMode) -> BoxFuture<'static, Result<(), AgentError>> {
        async { Ok(()) }.boxed()
    }
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        let options = self.config.choose(request);
        async move { Ok(options) }.boxed()
    }
    fn authenticate(&self, _: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        let failed = self.authentication_failure.load(Ordering::SeqCst);
        if !failed {
            self.auth_required.store(false, Ordering::SeqCst);
        }
        async move {
            if failed {
                Err(AgentError::Io("Sign-in failed".into()))
            } else {
                Ok(())
            }
        }
        .boxed()
    }
    fn close_session(
        &self,
        session: SessionId,
    ) -> BoxFuture<'static, Result<nocterm_ai::CloseSessionOutcome, AgentError>> {
        self.closed.lock().unwrap().push(session);
        let gate = self.close_gate.lock().unwrap().take();
        async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            Ok(nocterm_ai::CloseSessionOutcome::Closed)
        }
        .boxed()
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        let failed = self.shutdown_failure.load(Ordering::SeqCst);
        async move {
            if failed {
                Err(AgentError::Io("Process tree still alive".into()))
            } else {
                Ok(())
            }
        }
        .boxed()
    }
    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) struct Connector {
    pub(super) distinct_process_handles: AtomicBool,
    pub(super) commands: Arc<Commands>,
    pub(super) events: async_channel::Receiver<AgentEvent>,
    pub(super) connects: AtomicUsize,
    pub(super) roots: Mutex<
        Vec<(
            std::path::PathBuf,
            Option<nocterm_ai::sandbox::SandboxPolicy>,
        )>,
    >,
    pub(super) event_senders: Mutex<Vec<async_channel::Sender<AgentEvent>>>,
    pub(super) info: Mutex<AgentInfo>,
}
impl Connector {
    pub(super) fn use_distinct_process_handles(&self) {
        self.distinct_process_handles.store(true, Ordering::SeqCst);
    }
    pub(super) fn report_auth_methods(&self, methods: Vec<AuthMethod>) {
        self.info.lock().unwrap().auth_methods = methods;
    }
    pub(super) fn support_images(&self) {
        self.info.lock().unwrap().capabilities.image_prompts = true;
    }
}
impl AgentConnector for Connector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        self.roots
            .lock()
            .unwrap()
            .push((request.working_directory, request.sandbox));
        self.connects.fetch_add(1, Ordering::SeqCst);
        let commands: Arc<dyn AgentCommands> =
            if self.distinct_process_handles.load(Ordering::SeqCst) {
                Arc::new(super::process_commands::ProcessCommands::new(
                    self.commands.clone(),
                ))
            } else {
                self.commands.clone()
            };
        let info = self.info.lock().unwrap().clone();
        let (sender, events) = async_channel::bounded(256);
        self.event_senders.lock().unwrap().push(sender);
        async move {
            Ok(AgentConnection {
                commands,
                events,
                info,
            })
        }
        .boxed()
    }
}
pub(super) struct Bridge {
    pub(super) next: AtomicU64,
    pub(super) revoked: Arc<Mutex<Vec<u64>>>,
    pub(super) stops: AtomicUsize,
    pub(super) calls: async_channel::Receiver<BridgeCall>,
    pub(super) rejections: async_channel::Receiver<nocterm_ai::BridgeRejection>,
}
impl ToolBridge for Bridge {
    fn register(
        &self,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let revoked = self.revoked.clone();
        Ok(BridgeRegistration::new(
            id,
            nocterm_ai::BridgeLaunch {
                program: "/fake/nocterm".into(),
                args: vec!["agent-bridge".into()],
                env: vec![("NOCTERM_BRIDGE_TOKEN".into(), "test-token".into())],
            },
            Arc::new(move |id| revoked.lock().unwrap().push(id)),
        ))
    }
    fn calls(&self) -> async_channel::Receiver<BridgeCall> {
        self.calls.clone()
    }
    fn rejections(&self) -> async_channel::Receiver<nocterm_ai::BridgeRejection> {
        self.rejections.clone()
    }
    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}
