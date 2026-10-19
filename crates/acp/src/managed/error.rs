//! Allocation-free framing diagnostics retain original typed refusal causes.
use agent_client_protocol::managed::ManagedError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DecodeError<E: std::error::Error + 'static> {
    #[error(transparent)]
    Allocation(#[from] ManagedError<E>),
    #[error(transparent)]
    Native(#[from] nocterm_ai::model::json::deferred::JsonDecodeError),
    #[error(transparent)]
    Measurement(#[from] nocterm_ai::history::MeasurementError),
    #[error(transparent)]
    Model(#[from] nocterm_ai::model::diagnostic::ModelError),
    #[error("path contains invalid UTF-8 characters")]
    InvalidPath,
    #[error("invalid native JSON at byte {0}")]
    Syntax(usize),
    #[error("invalid native protocol field: {0}")]
    Field(&'static str),
}
