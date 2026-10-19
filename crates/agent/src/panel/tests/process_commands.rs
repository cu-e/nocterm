//! Distinct native process identity with the existing command script and counters.
use super::*;
use nocterm_ai::model::controls::ConfigOption;
use nocterm_ai::model::ids::{AuthMethodId, SessionId};
use nocterm_ai::model::session::{
    PromptOutcome, PromptRequest, SessionOpen, SessionOpened, SetConfigOption, SetMode,
};
pub(super) struct ProcessCommands(Arc<Commands>);
impl ProcessCommands {
    pub(super) fn new(commands: Arc<Commands>) -> Self {
        Self(commands)
    }
}
impl AgentCommands for ProcessCommands {
    fn new_session(
        &self,
        request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.0.new_session(request)
    }
    fn restore_session(
        &self,
        request: nocterm_ai::model::session::RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        self.0.restore_session(request)
    }
    fn prompt(
        &self,
        request: PromptRequest,
    ) -> BoxFuture<'static, Result<PromptOutcome, AgentError>> {
        self.0.prompt(request)
    }
    fn cancel(&self, session: SessionId) {
        self.0.cancel(session);
    }
    fn set_mode(&self, request: SetMode) -> BoxFuture<'static, Result<(), AgentError>> {
        self.0.set_mode(request)
    }
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        self.0.set_config_option(request)
    }
    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        self.0.authenticate(method)
    }
    fn close_session(
        &self,
        session: SessionId,
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
