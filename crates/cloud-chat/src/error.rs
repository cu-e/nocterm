#[derive(Debug, thiserror::Error)]
pub enum CloudChatError {
    #[error(transparent)]
    Source(#[from] nocterm_cloud_model::projection::ProjectionError),
    #[error(transparent)]
    History(#[from] nocterm_agent_runtime::history_admission::HistoryAdmissionError),
    #[error("The conversation viewer is gone.")]
    Closed,
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
            Self::Source(_) => Target::Unavailable,
        }
    }
}
