mod startup;
pub use startup::{
    ManagedConnectRequest, ManagedIsolationPaths, ManagedLaunch, ManagedStartup,
    ManagedStartupFacts, ManagedStartupRequest,
};
mod managed;
pub use crate::model::auth::PermissionResponder;
pub use crate::model::session::RestoreSession as RestoreSessionRequest;
use crate::model::{
    auth::AuthMethod,
    session::{AgentCapabilities, ExternalToolEndpoint},
};
#[cfg(any(test, feature = "test-support"))]
use crate::model::{auth::ProviderPermission, diagnostic::RemoteFault, notice::SessionNotice};
use crate::tools::TerminalCall;
use futures::channel::oneshot;
pub use managed::{
    ManagedAgentCommands, ManagedAgentConnection, ManagedAgentEvent, ManagedCommandsHandle,
    ManagedEventAcknowledgement, ManagedEventBarrier, ManagedEventReceiver, ManagedEventSource,
    ManagedPreparedPrompt, ManagedProcessCleanup, ManagedShutdownResult, ManagedShutdownWaiter,
    ManagedStartupHandle, ManagedWorkspaceHandle,
};
use std::{path::PathBuf, sync::Arc};
/// Shared startup cancellation. The connector acknowledges it by completing its future.
#[derive(Clone)]
pub struct ConnectionCancellation(crate::model::json::lifetime::Shared<ManagedCancellationState>);
#[derive(Default, Debug)]
struct CancellationState {
    cancelled: std::sync::atomic::AtomicBool,
    waker: futures::task::AtomicWaker,
}
struct ManagedCancellationState {
    state: CancellationState,
    allocation: crate::history::source::NativeAllocation,
}
impl std::fmt::Debug for ConnectionCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}
impl ConnectionCancellation {
    /// Mint the same cancellation state before native startup allocation begins.
    pub fn managed(
        context: &crate::history::source::SourceContext,
    ) -> Result<Self, crate::model::json::deferred::JsonDecodeError> {
        let allocation = context.reserve_native(
            std::alloc::Layout::from_size_align(
                crate::model::allocations::shared_inline_bytes::<ManagedCancellationState>(),
                std::mem::align_of::<ManagedCancellationState>().max(std::mem::align_of::<usize>()),
            )
            .map_err(|_| crate::model::json::deferred::JsonDecodeError::Arithmetic)?,
        )?;
        Ok(Self(crate::model::json::lifetime::Shared::new(
            ManagedCancellationState {
                state: CancellationState::default(),
                allocation,
            },
        )))
    }
    fn state(&self) -> &CancellationState {
        &self.0.state
    }
    pub fn cancel(&self) {
        self.state()
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.state().waker.wake();
    }
    pub fn is_cancelled(&self) -> bool {
        self.state()
            .cancelled
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn cancelled_managed(
        &self,
    ) -> Result<
        crate::history::source::ManagedFuture<()>,
        crate::model::json::deferred::JsonDecodeError,
    > {
        let token = self.clone();
        crate::history::source::ManagedFuture::new(
            self.0.allocation.source(),
            futures::future::poll_fn(move |cx| {
                token.state().waker.register(cx.waker());
                if token.is_cancelled() {
                    std::task::Poll::Ready(())
                } else {
                    std::task::Poll::Pending
                }
            }),
        )
    }
}

pub trait AgentConnector: Send + Sync + 'static {
    fn connect_managed(
        &self,
        request: ManagedConnectRequest,
    ) -> Result<
        crate::history::source::ManagedSourceFuture<
            Result<ManagedAgentConnection, crate::history::source::ManagedCommandError>,
        >,
        crate::model::json::deferred::NativeAllocationError,
    >;
}
#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub auth_methods: Vec<AuthMethod>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseSessionOutcome {
    Closed,
    Unsupported,
    TimedOut,
}

/// Test fixtures inject these legacy-shaped events; production uses `ManagedAgentEvent`.
#[cfg(any(test, feature = "test-support"))]
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

/// Oracle vocabulary for tests; production failures are `ManagedCommandError`.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("{0}")]
    Model(#[from] crate::model::diagnostic::ModelError),
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
    #[error("Agent error: {0}")]
    Rpc(#[source] RemoteFault),
    #[error("Agent exited ({code:?}): {stderr_tail}")]
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    #[error("{0}")]
    Io(String),
}
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
    /// A neutral local endpoint; only the live adapter constructs transport objects.
    pub fn endpoint(&self) -> ExternalToolEndpoint {
        ExternalToolEndpoint::LocalCommand {
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
    use crate::model::{auth::PermissionOutcome, ids::PermissionOptionId};
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
        let outcome = PermissionOutcome::Selected(PermissionOptionId::new("allow"));
        responder.respond(outcome.clone());
        assert_eq!(block_on(answer).unwrap(), outcome);
    }
}
