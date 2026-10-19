//! One connector fixture owns the original command state and event channels.
use super::*;
pub(super) struct Connector {
    script: nocterm_ai::testing::ScriptedCommands,
    events: async_channel::Receiver<AgentEvent>,
    connects: AtomicUsize,
    roots: Mutex<
        Vec<(
            std::path::PathBuf,
            Option<nocterm_ai::sandbox::SandboxPolicy>,
        )>,
    >,
    event_senders: Mutex<Vec<async_channel::Sender<AgentEvent>>>,
    info: Mutex<AgentInfo>,
    context: Mutex<Option<nocterm_ai::history::source::SourceContext>>,
}
impl Connector {
    pub(super) fn new(
        script: nocterm_ai::testing::ScriptedCommands,
        events: async_channel::Receiver<AgentEvent>,
    ) -> Self {
        Self {
            script,
            events,
            connects: AtomicUsize::new(0),
            roots: Default::default(),
            event_senders: Default::default(),
            context: Mutex::new(None),
            info: Mutex::new(AgentInfo {
                name: "Fake".into(),
                version: "1".into(),
                capabilities: Default::default(),
                auth_methods: vec![AuthMethod {
                    id: AuthMethodId::new("sign-in"),
                    label: "Account".into(),
                    description: None,
                    flow: AuthFlow::AgentManaged,
                }],
            }),
        }
    }
    /// A notice certified by the latest connection, as its event stream would deliver it.
    pub(super) fn notice(
        &self,
        session: SessionId,
        update: SessionUpdate,
    ) -> nocterm_ai::history::source::ManagedNotice {
        let context = self.context.lock().unwrap().clone().expect("connected");
        match nocterm_ai::testing::managed_event(
            &context,
            AgentEvent::Session(SessionNotice { session, update }),
        ) {
            Ok(nocterm_ai::connection::ManagedAgentEvent::Session(notice)) => notice,
            _ => panic!("fixture notice is certified"),
        }
    }
    pub(super) fn connect_count(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
    pub(super) fn roots(
        &self,
    ) -> Vec<(
        std::path::PathBuf,
        Option<nocterm_ai::sandbox::SandboxPolicy>,
    )> {
        self.roots.lock().unwrap().clone()
    }
    pub(super) fn events(&self) -> async_channel::Receiver<AgentEvent> {
        self.events.clone()
    }
    pub(super) fn process_events(&self, index: usize) -> async_channel::Sender<AgentEvent> {
        self.event_senders.lock().unwrap()[index].clone()
    }

    pub(super) fn report_auth_methods(&self, methods: Vec<AuthMethod>) {
        self.info.lock().unwrap().auth_methods = methods;
    }
    pub(super) fn support_images(&self) {
        self.info.lock().unwrap().capabilities.content.image = true;
    }
}
impl AgentConnector for Connector {
    fn connect_managed(
        &self,
        request: nocterm_ai::connection::ManagedConnectRequest,
    ) -> Result<
        nocterm_ai::history::source::ManagedSourceFuture<
            Result<
                nocterm_ai::connection::ManagedAgentConnection,
                nocterm_ai::history::source::ManagedCommandError,
            >,
        >,
        nocterm_ai::model::json::deferred::NativeAllocationError,
    > {
        use nocterm_ai::{
            connection::ManagedAgentConnection,
            history::source::{ManagedCommandError, ManagedSourceFuture},
            model::{json::deferred::JsonDecodeError, native::NativeCopies},
        };
        let context = request.context().clone();
        let lifetime = request.lifetime();
        *self.context.lock().unwrap() = Some(context.clone());
        let copy_failure = context.prepare_error::<JsonDecodeError>(0)?;
        let certify_failure = context
            .prepare_error::<nocterm_ai::history::source::SourceSemanticRefusal<AgentInfo>>(0)?;
        let event_failure = context.prepare_error::<JsonDecodeError>(0)?;
        // The recorded roots are explicit test observations. Production work keeps
        // the original guarded workspace handle throughout the startup future.
        self.roots.lock().unwrap().push((
            request.workspace().root.clone(),
            request.workspace().sandbox.clone(),
        ));
        self.connects.fetch_add(1, Ordering::SeqCst);
        let native = NativeCopies::new(context.ledger()).info(&self.info.lock().unwrap());
        let (sender, events) = async_channel::bounded(256);
        self.event_senders.lock().unwrap().push(sender);
        // Each connection is a distinct native process sharing the script's observations.
        let commands = self.script.fresh_handle(&lifetime)?;
        let events = nocterm_ai::testing::managed_events(&context, lifetime.clone(), events);
        ManagedSourceFuture::new(lifetime, async move {
            let native =
                native.map_err(|error| ManagedCommandError::Native(copy_failure.capture(error)))?;
            let info = context
                .certify_info(native)
                .map_err(|error| ManagedCommandError::Native(certify_failure.capture(error)))?;
            let events = events
                .map_err(|error| ManagedCommandError::Native(event_failure.capture(error)))?;
            drop(request);
            Ok(ManagedAgentConnection::new(info, commands, events))
        })
    }
}
