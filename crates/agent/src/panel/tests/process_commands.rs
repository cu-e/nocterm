//! Distinct native process identity with the existing command script and counters.
use super::*;
pub(super) struct ProcessCommands(Arc<Commands>);
impl ProcessCommands {
    pub(super) fn new(commands: Arc<Commands>) -> Self {
        Self(commands)
    }
}
impl AgentCommands for ProcessCommands {
    fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        self.0.new_session(request)
    }
    fn restore_session(
        &self,
        request: nocterm_ai::RestoreSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        self.0.restore_session(request)
    }
    fn prompt(
        &self,
        request: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>> {
        self.0.prompt(request)
    }
    fn cancel(&self, session: acp::SessionId) {
        self.0.cancel(session);
    }
    fn set_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        self.0.set_mode(request)
    }
    fn set_config_option(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<Vec<acp::SessionConfigOption>, AgentError>> {
        self.0.set_config_option(request)
    }
    fn authenticate(
        &self,
        method: acp::AuthMethodId,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        self.0.authenticate(method)
    }
    fn close_session(
        &self,
        session: acp::SessionId,
    ) -> BoxFuture<'static, Result<nocterm_ai::CloseSessionOutcome, AgentError>> {
        self.0.close_session(session)
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.0.shutdown_gracefully()
    }
    fn shutdown(&self) {
        self.0.shutdown();
    }
}
