use crate::{acp, registry::AgentLaunch, tools::TerminalCall};
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
    pub resources: nocterm_settings::AgentResourceSettings,
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
    pub capabilities: acp::AgentCapabilities,
    pub auth_methods: Vec<acp::AuthMethod>,
}
pub struct AgentConnection {
    pub info: AgentInfo,
    pub commands: Arc<dyn AgentCommands>,
    pub events: async_channel::Receiver<AgentEvent>,
}
pub trait AgentCommands: Send + Sync {
    fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>>;
    /// Reopens a session the agent held before, with `session/resume` or
    /// `session/load`, whichever the agent offers, or opens a copy of it with
    /// `session/fork` when `request.fork` is set. Agents offering none of
    /// these answer with an error, and callers start a new session instead.
    fn restore_session(
        &self,
        _request: RestoreSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        Box::pin(async {
            Err(AgentError::RestoreUnavailable(
                "The agent cannot reopen earlier chats.".into(),
            ))
        })
    }
    fn prompt(
        &self,
        request: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>>;
    fn cancel(&self, session: acp::SessionId);
    fn set_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<(), AgentError>>;
    fn set_config_option(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<Vec<acp::SessionConfigOption>, AgentError>>;
    fn authenticate(&self, method: acp::AuthMethodId)
    -> BoxFuture<'static, Result<(), AgentError>>;
    /// Completes only after the agent acknowledges close and earlier events are drained.
    fn close_session(
        &self,
        session: acp::SessionId,
    ) -> BoxFuture<'static, Result<CloseSessionOutcome, AgentError>>;
    /// Stops the connection, awaiting bounded process-tree cleanup.
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        Box::pin(async { Ok(()) })
    }
    fn shutdown(&self);
}
/// A session to reopen.
#[derive(Clone, Debug)]
pub struct RestoreSessionRequest {
    pub session_id: acp::SessionId,
    pub cwd: PathBuf,
    pub mcp_servers: Vec<acp::McpServer>,
    /// Open a new session that starts with this one's history instead.
    pub fork: bool,
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
    Session(acp::SessionNotification),
    Permission {
        request: acp::RequestPermissionRequest,
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
pub struct PermissionResponder(Option<oneshot::Sender<acp::RequestPermissionOutcome>>);

impl PermissionResponder {
    /// A responder and the receiver its answer arrives on.
    pub fn channel() -> (Self, oneshot::Receiver<acp::RequestPermissionOutcome>) {
        let (sender, receiver) = oneshot::channel();
        (Self(Some(sender)), receiver)
    }

    pub fn respond(mut self, outcome: acp::RequestPermissionOutcome) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(outcome);
        }
    }
}

impl Drop for PermissionResponder {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(acp::RequestPermissionOutcome::Cancelled);
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Agent process cleanup could not be confirmed: {0}")]
    CleanupUnconfirmed(String),
    #[error("Authentication required: {0}")]
    AuthRequired(String),
    #[error("{0}")]
    RestoreUnavailable(String),
    #[error("ACP error: {0}")]
    Rpc(acp::Error),
    #[error("Agent exited ({code:?}): {stderr_tail}")]
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    #[error("{0}")]
    Io(String),
}
pub type ConnectError = AgentError;
pub trait ToolBridge: Send + Sync + 'static {
    fn register(&self) -> Result<BridgeRegistration, String>;
    fn calls(&self) -> async_channel::Receiver<BridgeCall>;
    fn stop(&self);
}
pub struct BridgeRegistration {
    pub id: u64,
    pub endpoint: String,
    pub token: String,
    revoke: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}
impl BridgeRegistration {
    pub fn new(
        id: u64,
        endpoint: String,
        token: String,
        revoke: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Self {
        Self {
            id,
            endpoint,
            token,
            revoke: Some(revoke),
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
pub struct BridgeCall {
    pub registration_id: u64,
    pub call: TerminalCall,
    pub respond: oneshot::Sender<Result<serde_json::Value, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn a_dropped_responder_answers_cancelled() {
        let (responder, answer) = PermissionResponder::channel();
        drop(responder);
        assert_eq!(
            block_on(answer).unwrap(),
            acp::RequestPermissionOutcome::Cancelled
        );
    }

    #[test]
    fn a_responder_sends_its_outcome_once() {
        let (responder, answer) = PermissionResponder::channel();
        let outcome =
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new("allow"));
        responder.respond(outcome.clone());
        assert_eq!(block_on(answer).unwrap(), outcome);
    }
}
