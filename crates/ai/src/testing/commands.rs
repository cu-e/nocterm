//! Scripted native commands for process-lifecycle tests, shared by service and feature fixtures.
//! The script owns close and shutdown. Session operations go to an optional [`SessionScript`];
//! unscripted ones refuse with a typed fixture error rather than inventing a successful reply.
use super::connection::ManagedConnectFuture;
use crate::{
    CloseSessionOutcome,
    connection::{
        ManagedAgentCommands, ManagedAgentConnection, ManagedAgentEvent, ManagedCommandsHandle,
        ManagedConnectRequest, ManagedEventReceiver, ManagedEventSource, ManagedPreparedPrompt,
    },
    history::{
        physical::DispatchPayload,
        source::{
            ManagedCommandError, ManagedOpened, ManagedSourceFuture, SourceContext, SourceLifetime,
        },
    },
    model::{
        controls::ConfigOption,
        ids::{AuthMethodId, SessionId},
        json::deferred::NativeAllocationError,
        native::{NativeCopies, NativeOwned},
        session::{ConfigurationSelection, ModeSelection, RestoreSession, SessionOpen},
    },
};
use futures::channel::oneshot;
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
/// The original failures a script reports through its commands.
#[derive(Debug, thiserror::Error)]
pub enum ScriptedFailure {
    #[error("unused scripted operation")]
    Unused,
    #[error("cleanup was not confirmed")]
    Cleanup,
}
#[derive(Default)]
struct State {
    closes: AtomicUsize,
    shutdowns: AtomicUsize,
    hang_shutdown: AtomicBool,
    fail_shutdown: AtomicBool,
    /// Ungated closes finish at once instead of never answering.
    complete_closes: AtomicBool,
    closed: Mutex<Vec<SessionId>>,
    close_gate: Mutex<Option<oneshot::Receiver<()>>>,
    shutdown_gate: Mutex<Option<oneshot::Receiver<()>>>,
}
/// A scripted operation admitted under the connection's original Source.
pub type ScriptedOperation<T> =
    Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>;
type Operation<T> = ScriptedOperation<T>;
/// Session operations a fixture scripts on top of the shared close and shutdown lifecycle.
/// Each default refuses with a typed fixture error.
pub trait SessionScript: Send + Sync + 'static {
    fn new_session(&self, _: &SessionOpen, lifetime: &SourceLifetime) -> Operation<ManagedOpened> {
        unused(lifetime)
    }
    fn restore_session(
        &self,
        _: &RestoreSession,
        lifetime: &SourceLifetime,
    ) -> Operation<ManagedOpened> {
        unused(lifetime)
    }
    fn prepare_prompt(
        &self,
        _: &DispatchPayload,
        lifetime: &SourceLifetime,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        let slot = lifetime
            .context_ref()
            .prepare_error::<ScriptedFailure>(0)
            .map_err(ManagedCommandError::Admission)?;
        Err(ManagedCommandError::Native(
            slot.capture(ScriptedFailure::Unused),
        ))
    }
    fn cancel(&self, _: &SessionId) -> Result<(), ManagedCommandError> {
        Ok(())
    }
    fn set_mode(&self, _: &ModeSelection, lifetime: &SourceLifetime) -> Operation<()> {
        unused(lifetime)
    }
    fn set_config_option(
        &self,
        _: &ConfigurationSelection,
        lifetime: &SourceLifetime,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>> {
        unused(lifetime)
    }
    fn authenticate(&self, _: &AuthMethodId, lifetime: &SourceLifetime) -> Operation<()> {
        unused(lifetime)
    }
}
struct Unscripted;
impl SessionScript for Unscripted {}
fn unused<T: Send + 'static>(lifetime: &SourceLifetime) -> Operation<T> {
    let slot = lifetime.context_ref().prepare_error::<ScriptedFailure>(0)?;
    ManagedSourceFuture::new(lifetime.clone(), async move {
        Err(ManagedCommandError::Native(
            slot.capture(ScriptedFailure::Unused),
        ))
    })
}
/// One observable script; connections made through [`Self::connect`] share one command handle.
#[derive(Clone)]
pub struct ScriptedCommands {
    state: Arc<State>,
    sessions: Arc<dyn SessionScript>,
    handle: Arc<Mutex<Option<ManagedCommandsHandle>>>,
    owner: Arc<Mutex<Weak<()>>>,
    events: Arc<Mutex<Weak<()>>>,
}
impl Default for ScriptedCommands {
    fn default() -> Self {
        Self::with_sessions(Unscripted)
    }
}
/// Weak observation of the script's native command owner, for retention assertions.
pub struct CommandsWitness(Arc<Mutex<Weak<()>>>);
impl CommandsWitness {
    pub fn retained(&self) -> bool {
        self.0.lock().expect("script owner").strong_count() > 0
    }
}
impl ScriptedCommands {
    /// Session operations go to `sessions`; close and shutdown stay with the script.
    pub fn with_sessions(sessions: impl SessionScript) -> Self {
        Self {
            state: Arc::default(),
            sessions: Arc::new(sessions),
            handle: Arc::default(),
            owner: Arc::default(),
            events: Arc::default(),
        }
    }
    pub fn closes(&self) -> usize {
        self.state.closes.load(Ordering::SeqCst)
    }
    /// Graceful, final and synchronous shutdown invocations together.
    pub fn shutdowns(&self) -> usize {
        self.state.shutdowns.load(Ordering::SeqCst)
    }
    /// Sessions the agent was asked to close, oldest first.
    pub fn closed_sessions(&self) -> Vec<SessionId> {
        self.state.closed.lock().expect("closed sessions").clone()
    }
    /// Closes that are not gated answer `Closed` immediately.
    pub fn complete_closes(&self) {
        self.state.complete_closes.store(true, Ordering::SeqCst);
    }
    pub fn fail_shutdown(&self) {
        self.state.fail_shutdown.store(true, Ordering::SeqCst);
    }
    /// Later shutdown attempts succeed again.
    pub fn repair_shutdown(&self) {
        self.state.fail_shutdown.store(false, Ordering::SeqCst);
    }
    /// Every shutdown attempt starts and never finishes.
    pub fn hang_shutdown(&self) {
        self.state.hang_shutdown.store(true, Ordering::SeqCst);
    }
    /// The next close holds until the returned sender fires.
    pub fn gate_close(&self) -> oneshot::Sender<()> {
        let (sender, gate) = oneshot::channel();
        *self.state.close_gate.lock().expect("close gate") = Some(gate);
        sender
    }
    /// The next shutdown holds until the returned sender fires.
    pub fn gate_shutdown(&self) -> oneshot::Sender<()> {
        let (sender, gate) = oneshot::channel();
        *self.state.shutdown_gate.lock().expect("shutdown gate") = Some(gate);
        sender
    }
    pub fn witness(&self) -> CommandsWitness {
        CommandsWitness(self.owner.clone())
    }
    /// Weak observation of the latest connection's event source.
    pub fn events_witness(&self) -> CommandsWitness {
        CommandsWitness(self.events.clone())
    }
    /// Forget the cached handle so a witness can observe only external owners.
    pub fn release_handle(&self) {
        self.handle.lock().expect("script handle").take();
    }
    /// The script's one command handle, created under `lifetime`'s Source on first use.
    pub fn handle(
        &self,
        lifetime: &SourceLifetime,
    ) -> Result<ManagedCommandsHandle, NativeAllocationError> {
        let mut cached = self.handle.lock().expect("script handle");
        if let Some(commands) = cached.as_ref() {
            return Ok(commands.clone());
        }
        let commands = self.fresh_handle(lifetime)?;
        *cached = Some(commands.clone());
        Ok(commands)
    }
    /// A distinct command handle for one native process under `lifetime`'s Source; it shares
    /// the script's counters, gates and session operations but not its cached handle.
    pub fn fresh_handle(
        &self,
        lifetime: &SourceLifetime,
    ) -> Result<ManagedCommandsHandle, NativeAllocationError> {
        let token = Arc::new(());
        *self.owner.lock().expect("script owner") = Arc::downgrade(&token);
        ManagedCommandsHandle::new_with_lifetime(
            lifetime.context_ref(),
            lifetime.clone(),
            Commands {
                state: self.state.clone(),
                sessions: self.sessions.clone(),
                lifetime: lifetime.clone(),
                _owner: token,
            },
        )
    }
    /// Connects under `request`'s original Source with a certified default agent.
    pub fn connect(&self, request: ManagedConnectRequest) -> ManagedConnectFuture {
        let context = request.context().clone();
        let lifetime = request.lifetime();
        let copy = context.prepare_error::<crate::model::json::deferred::JsonDecodeError>(0)?;
        let certify = context.prepare_error::<crate::history::source::SourceSemanticRefusal<
            crate::connection::AgentInfo,
        >>(0)?;
        let events_failure =
            context.prepare_error::<crate::model::json::deferred::JsonDecodeError>(0)?;
        let native = NativeCopies::new(context.ledger()).info(&crate::connection::AgentInfo {
            name: "Scripted".into(),
            version: "1".into(),
            capabilities: Default::default(),
            auth_methods: Vec::new(),
        });
        let commands = self.handle(&lifetime)?;
        let token = Arc::new(());
        *self.events.lock().expect("script events") = Arc::downgrade(&token);
        let events = ManagedEventReceiver::with_lifetime(
            &context,
            lifetime.clone(),
            Silent { _token: token },
        );
        ManagedSourceFuture::new(lifetime, async move {
            let native =
                native.map_err(|error| ManagedCommandError::Native(copy.capture(error)))?;
            let info = context
                .certify_info(native)
                .map_err(|error| ManagedCommandError::Native(certify.capture(error)))?;
            let events = events
                .map_err(|error| ManagedCommandError::Native(events_failure.capture(error)))?;
            drop(request);
            Ok(ManagedAgentConnection::new(info, commands, events))
        })
    }
}
struct Silent {
    _token: Arc<()>,
}
impl ManagedEventSource for Silent {
    fn poll_event(&mut self, _: &mut Context<'_>) -> Poll<Option<ManagedAgentEvent>> {
        Poll::Pending
    }
}
struct Commands {
    state: Arc<State>,
    sessions: Arc<dyn SessionScript>,
    lifetime: SourceLifetime,
    _owner: Arc<()>,
}
impl Commands {
    fn context(&self) -> &SourceContext {
        self.lifetime.context_ref()
    }
    fn shutdown_attempt(&self) -> Operation<()> {
        self.state.shutdowns.fetch_add(1, Ordering::SeqCst);
        let slot = self.context().prepare_error::<ScriptedFailure>(0)?;
        let gate = self
            .state
            .shutdown_gate
            .lock()
            .expect("shutdown gate")
            .take();
        let hang = self.state.hang_shutdown.load(Ordering::SeqCst);
        let fail = self.state.fail_shutdown.load(Ordering::SeqCst);
        ManagedSourceFuture::new(self.lifetime.clone(), async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            } else if hang {
                futures::future::pending::<()>().await;
            }
            if fail {
                Err(ManagedCommandError::CleanupUnconfirmed(
                    slot.capture(ScriptedFailure::Cleanup),
                ))
            } else {
                Ok(())
            }
        })
    }
}
impl ManagedAgentCommands for Commands {
    fn new_session(&self, request: &SessionOpen) -> Operation<ManagedOpened> {
        self.sessions.new_session(request, &self.lifetime)
    }
    fn restore_session(&self, request: &RestoreSession) -> Operation<ManagedOpened> {
        self.sessions.restore_session(request, &self.lifetime)
    }
    fn prepare_prompt(
        &self,
        payload: &DispatchPayload,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        self.sessions.prepare_prompt(payload, &self.lifetime)
    }
    fn cancel(&self, session: &SessionId) -> Result<(), ManagedCommandError> {
        self.sessions.cancel(session)
    }
    fn set_mode(&self, request: &ModeSelection) -> Operation<()> {
        self.sessions.set_mode(request, &self.lifetime)
    }
    fn set_config_option(
        &self,
        request: &ConfigurationSelection,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>> {
        self.sessions.set_config_option(request, &self.lifetime)
    }
    fn authenticate(&self, method: &AuthMethodId) -> Operation<()> {
        self.sessions.authenticate(method, &self.lifetime)
    }
    fn close_session(&self, session: &SessionId) -> Operation<CloseSessionOutcome> {
        self.state.closes.fetch_add(1, Ordering::SeqCst);
        self.state
            .closed
            .lock()
            .expect("closed sessions")
            .push(session.clone());
        let gate = self.state.close_gate.lock().expect("close gate").take();
        let complete = self.state.complete_closes.load(Ordering::SeqCst);
        ManagedSourceFuture::new(self.lifetime.clone(), async move {
            match gate {
                Some(gate) => {
                    let _ = gate.await;
                    Ok(CloseSessionOutcome::Closed)
                }
                None if complete => Ok(CloseSessionOutcome::Closed),
                None => futures::future::pending().await,
            }
        })
    }
    fn shutdown_gracefully(&self) -> Operation<()> {
        self.shutdown_attempt()
    }
    fn shutdown_finally(&self) -> Operation<()> {
        self.shutdown_attempt()
    }
    fn shutdown(&self) {
        self.state.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}
