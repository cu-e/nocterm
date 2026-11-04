#[derive(Debug, thiserror::Error)]
pub enum CloudChatError {
    #[error(transparent)]
    Source(#[from] nocterm_cloud_model::projection::ProjectionError),
    #[error(transparent)]
    History(#[from] nocterm_agent_runtime::history_admission::HistoryAdmissionError),
    #[error("The conversation viewer is gone.")]
    Closed,
    /// The runner refused the command for good; its body was retired.
    #[error("The runner refused the command.")]
    Rejected,
    /// The hub or the runner did not answer the command's delivery; it stays for a retry.
    #[error("The hub or the runner did not answer.")]
    Unreachable,
}
impl From<crate::pending::CommandFailure> for CloudChatError {
    fn from(failure: crate::pending::CommandFailure) -> Self {
        match failure {
            crate::pending::CommandFailure::Source(error) => Self::Source(error),
            crate::pending::CommandFailure::Rejected => Self::Rejected,
            crate::pending::CommandFailure::Unreachable => Self::Unreachable,
        }
    }
}
impl CloudChatError {
    pub(crate) fn into_conversation(self) -> nocterm_agent_chat::conversation::ConversationError {
        use nocterm_agent_chat::conversation::ConversationError as Target;
        use nocterm_cloud_model::projection::ProjectionError as Source;
        match self {
            Self::Closed
            | Self::Source(
                Source::Stale | Source::WrongSource | Source::WrongRun | Source::Rollback,
            ) => Target::Stale,
            Self::History(_) | Self::Source(Source::Capacity) => Target::Capacity,
            Self::Source(Source::OutcomeUnknown) => Target::OutcomeUnknown,
            Self::Unreachable => Target::Unreachable,
            Self::Rejected => Target::Rejected,
            Self::Source(_) => Target::Unavailable,
        }
    }
}
