use nocterm_cloud_conversation_model::CloudConversationError;

#[derive(Debug, thiserror::Error)]
pub enum CloudChatError {
    #[error(transparent)]
    Conversation(#[from] CloudConversationError),
    #[error(transparent)]
    History(#[from] nocterm_agent_runtime::history_admission::HistoryAdmissionError),
    #[error("The conversation viewer is gone.")]
    Closed,
    /// The hub or the runner refused the command for good.
    #[error("The runner refused the command.")]
    Rejected,
    /// Neither the hub nor the runner answered the command before it expired.
    #[error("The hub or the runner did not answer.")]
    Unreachable,
}
impl CloudChatError {
    pub(crate) fn into_conversation(self) -> nocterm_agent_chat::conversation::ConversationError {
        use nocterm_agent_chat::conversation::ConversationError as Target;
        match self {
            Self::Closed
            | Self::Conversation(CloudConversationError::Stale | CloudConversationError::Ended) => {
                Target::Stale
            }
            Self::History(_) | Self::Conversation(CloudConversationError::Capacity) => {
                Target::Capacity
            }
            Self::Unreachable | Self::Conversation(CloudConversationError::Unavailable) => {
                Target::Unreachable
            }
            Self::Rejected | Self::Conversation(CloudConversationError::Refused(_)) => {
                Target::Rejected
            }
            Self::Conversation(CloudConversationError::NoModel) => Target::ModelUnavailable,
            Self::Conversation(error @ CloudConversationError::LargePromptUnsupported(_)) => {
                Target::LargePromptUnsupported(nocterm_core::error::SharedCause::new(error))
            }
            Self::Conversation(CloudConversationError::ProviderBusy) => Target::ProviderBusy,
            Self::Conversation(CloudConversationError::Servers(refusal)) => {
                Target::Servers(crate::servers::problem(refusal))
            }
            Self::Conversation(_) => Target::Unavailable,
        }
    }
}
