//! Prompt completion carries the exact turn authority that launched its request.
use super::SessionAuthority;
use nocterm_ai::model::session::{PromptOutcome, PromptRequest};
use nocterm_ai::{AgentCommands, AgentError};
use std::sync::Arc;
pub(in crate::thread) struct PromptOperation {
    authority: SessionAuthority,
    response: futures::future::BoxFuture<'static, Result<PromptOutcome, AgentError>>,
}
impl PromptOperation {
    pub(in crate::thread) fn new(
        authority: SessionAuthority,
        commands: Arc<dyn AgentCommands>,
        request: PromptRequest,
    ) -> Self {
        Self {
            authority,
            response: commands.prompt(request),
        }
    }
    pub(in crate::thread) async fn run(
        self,
    ) -> (SessionAuthority, Result<PromptOutcome, AgentError>) {
        (self.authority, self.response.await)
    }
}
