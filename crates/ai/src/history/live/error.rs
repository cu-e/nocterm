//! Live owner failures keep their typed transcript, retention and codec causes.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error(transparent)]
    Dispatch(#[from] crate::history::dispatch::DispatchError),
    #[error("live update belongs to a stale epoch or revision")]
    Stale,
    #[error("live document has no installed payload")]
    MissingPayload,
    #[error("live owner received a role of another kind or document")]
    Role,
    #[error(transparent)]
    Physical(#[from] crate::history::physical::PhysicalError),
    #[error(transparent)]
    Retention(#[from] crate::model::retention::RetentionError),
    #[error(transparent)]
    History(#[from] crate::history::HistoryCodecError),
}
