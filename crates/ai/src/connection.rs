use crate::{acp, registry::AgentLaunch, tools::TerminalCall};
use futures::{channel::oneshot, future::BoxFuture};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Debug)]
pub struct ConnectRequest {
    pub launch: AgentLaunch,
    pub working_directory: PathBuf,
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
    fn close_session(&self, session: acp::SessionId);
    fn shutdown(&self);
}
pub enum AgentEvent {
    Session(acp::SessionNotification),
    Permission {
        request: acp::RequestPermissionRequest,
        respond: oneshot::Sender<acp::RequestPermissionOutcome>,
    },
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Authentication required: {0}")]
    AuthRequired(String),
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
