//! Prompt completion carries the exact turn authority that launched its request.
use super::SessionAuthority;
use nocterm_ai::{AgentCommands, AgentError, acp};
use std::sync::Arc;
pub(in crate::thread) struct PromptOperation {
    authority: SessionAuthority,
    response: futures::future::BoxFuture<'static, Result<acp::PromptResponse, AgentError>>,
}
impl PromptOperation {
    pub(in crate::thread) fn new(
        authority: SessionAuthority,
        commands: Arc<dyn AgentCommands>,
        request: acp::PromptRequest,
    ) -> Self {
        Self {
            authority,
            response: commands.prompt(request),
        }
    }
    pub(in crate::thread) async fn run(
        self,
    ) -> (SessionAuthority, Result<acp::PromptResponse, AgentError>) {
        (self.authority, self.response.await)
    }
}
