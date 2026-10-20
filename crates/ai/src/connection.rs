use crate::{
    agents::AgentLaunch,
    model::{
        auth::{AuthMethod, PermissionOutcome, ProviderPermission},
        controls::ConfigOption,
        diagnostic::ModelError,
        ids::{AuthMethodId, SessionId},
        session::{
            AgentCapabilitiesSummary, ExternalToolEndpoint, PromptOutcome, PromptRequest,
            RemoteError, RestoreSession, SessionNotice, SessionOpen, SessionOpened,
            SetConfigOption, SetMode,
        },
    },
    tools::TerminalCall,
};
use futures::{channel::oneshot, future::BoxFuture};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Debug)]
pub struct ConnectRequest {
    pub launch: AgentLaunch,
    pub working_directory: PathBuf,
    /// The host can open an interactive authentication command in a dedicated PTY.
    pub terminal_auth: bool,
    /// Run the agent isolated under this policy; `None` runs it directly.
    pub sandbox: Option<crate::sandbox::SandboxPolicy>,
    /// Limits for the complete agent process tree.
    pub resources: crate::AgentResourceSettings,
    /// Cancellation must return only after any startup process tree is stopped.
    pub cancellation: ConnectionCancellation,
}

/// Shared startup cancellation. The connector acknowledges it by completing its future.
#[derive(Clone, Default, Debug)]
pub struct ConnectionCancellation(std::sync::Arc<CancellationState>);
#[derive(Default, Debug)]
struct CancellationState {
    cancelled: std::sync::atomic::AtomicBool,
    waker: futures::task::AtomicWaker,
}
impl ConnectionCancellation {
    pub fn cancel(&self) {
        self.0
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.0.waker.wake();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn cancelled(&self) -> BoxFuture<'static, ()> {
        let token = self.clone();
        Box::pin(futures::future::poll_fn(move |cx| {
            token.0.waker.register(cx.waker());
            if token.is_cancelled() {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        }))
    }
}

pub trait AgentConnector: Send + Sync + 'static {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, ConnectError>>;
}
#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    pub capabilities: AgentCapabilitiesSummary,
    pub auth_methods: Vec<AuthMethod>,
}
pub struct AgentConnection {
    pub info: AgentInfo,
    pub commands: Arc<dyn AgentCommands>,
    pub events: async_channel::Receiver<AgentEvent>,
}
pub trait AgentCommands: Send + Sync {
    fn new_session(
        &self,
        request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>>;
    /// Reopens a session the agent held before, with `session/resume` or
    /// `session/load`, whichever the agent offers, or opens a copy of it with
    /// `session/fork` when `request.fork` is set. Agents offering none of
    /// these answer with an error, and callers start a new session instead.
    fn restore_session(
        &self,
        _request: RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        Box::pin(async {
            Err(AgentError::RestoreUnavailable(
                "The agent cannot reopen earlier chats.".into(),
            ))
        })
    }
    fn prompt(
        &self,
        request: PromptRequest,
    ) -> BoxFuture<'static, Result<PromptOutcome, AgentError>>;
    fn cancel(&self, session: SessionId);
    fn set_mode(&self, request: SetMode) -> BoxFuture<'static, Result<(), AgentError>>;
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>>;
    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>>;
    /// Completes only after the agent acknowledges close and earlier events are drained.
    fn close_session(
        &self,
        session: SessionId,
    ) -> BoxFuture<'static, Result<CloseSessionOutcome, AgentError>>;
    /// Stops the connection, awaiting bounded process-tree cleanup.
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        Box::pin(async { Ok(()) })
    }
    fn shutdown(&self);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseSessionOutcome {
    Closed,
    Unsupported,
    TimedOut,
}

pub enum AgentEvent {
    /// FIFO fence: the foreground consumer acknowledges all earlier events.
    Barrier(oneshot::Sender<()>),
    Session(SessionNotice),
    Permission {
        request: ProviderPermission,
        respond: PermissionResponder,
    },
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
}

/// The answer to one permission request.
///
/// Dropping it unanswered answers `Cancelled`, so a request that nobody
/// handles, or whose thread or dialog is closed, never leaves the agent
/// waiting.
#[derive(Debug)]
pub struct PermissionResponder(Option<oneshot::Sender<PermissionOutcome>>);

impl PermissionResponder {
    /// A responder and the receiver its answer arrives on.
    pub fn channel() -> (Self, oneshot::Receiver<PermissionOutcome>) {
        let (sender, receiver) = oneshot::channel();
        (Self(Some(sender)), receiver)
    }

    pub fn respond(mut self, outcome: PermissionOutcome) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(outcome);
        }
    }
}

impl Drop for PermissionResponder {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(PermissionOutcome::Cancelled);
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("{0}")]
    Workspace(#[from] crate::workspace::WorkdirError),
    #[error("{0}")]
    Sandbox(#[from] crate::sandbox::SandboxLaunchError),
    #[error("Agent process cleanup could not be confirmed: {0}")]
    CleanupUnconfirmed(String),
    #[error("Authentication required: {0}")]
    AuthRequired(String),
    #[error("{0}")]
    RestoreUnavailable(String),
    #[error("ACP error: {0}")]
    Rpc(RemoteError),
    #[error("{0}")]
    Unsupported(#[from] ModelError),
    #[error("Agent exited ({code:?}): {stderr_tail}")]
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    #[error("{0}")]
    Io(String),
}
pub type ConnectError = AgentError;
#[derive(Debug, thiserror::Error)]
pub enum BridgeRegistrationError {
    #[error("AI is disabled.")]
    AiDisabled,
    #[error("Bridge error: {0}")]
    Failed(String),
}

pub trait ToolBridge: Send + Sync + 'static {
    fn register(&self) -> Result<BridgeRegistration, BridgeRegistrationError>;
    fn calls(&self) -> async_channel::Receiver<BridgeCall>;
    fn rejections(&self) -> async_channel::Receiver<BridgeRejection> {
        let (sender, receiver) = async_channel::bounded(1);
        sender.close();
        receiver
    }
    fn stop(&self);
}
/// How an agent starts the relay to a chat's bridge. The adapter that owns the
/// bridge decides the command; the chat only hands it to the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}
pub struct BridgeRegistration {
    pub id: u64,
    launch: BridgeLaunch,
    revoke: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}
impl BridgeRegistration {
    pub fn new(id: u64, launch: BridgeLaunch, revoke: Arc<dyn Fn(u64) + Send + Sync>) -> Self {
        Self {
            id,
            launch,
            revoke: Some(revoke),
        }
    }
    pub fn launch(&self) -> &BridgeLaunch {
        &self.launch
    }
    /// The chat's terminal tools server, as the agent should start it.
    pub fn endpoint(&self) -> ExternalToolEndpoint {
        ExternalToolEndpoint {
            name: crate::tool_display::bridge_server_name(self.id),
            launch: self.launch.clone(),
        }
    }
    pub fn revoke(&mut self) {
        if let Some(revoke) = self.revoke.take() {
            revoke(self.id);
        }
    }
}
impl Drop for BridgeRegistration {
    fn drop(&mut self) {
        self.revoke();
    }
}
pub struct BridgeRejection {
    pub registration_id: u64,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub error: String,
}

pub struct BridgeCall {
    pub registration_id: u64,
    pub call: TerminalCall,
    pub arguments: Option<serde_json::Value>,
    /// Assigned at chat intake; identifies a particular request within its turn.
    pub display_token: Option<(u64, u64)>,
    pub respond: oneshot::Sender<Result<serde_json::Value, crate::tools::ToolFailure>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn a_dropped_responder_answers_cancelled() {
        let (responder, answer) = PermissionResponder::channel();
        drop(responder);
        assert_eq!(block_on(answer).unwrap(), PermissionOutcome::Cancelled);
    }

    #[test]
    fn a_responder_sends_its_outcome_once() {
        let (responder, answer) = PermissionResponder::channel();
        let outcome = PermissionOutcome::Selected("allow".into());
        responder.respond(outcome.clone());
        assert_eq!(block_on(answer).unwrap(), outcome);
    }
}
