//! Semantic failures of the bounded saved-chat format.
use nocterm_core::SharedError;

#[derive(Clone, Debug, thiserror::Error)]
pub enum HistoryCodecError {
    #[error("Saved chat formatted key measurement failed: {0}")]
    Formatting(#[from] std::fmt::Error),
    /// A pure native format/shape validator rejected the document. Real I/O
    /// belongs to storage Read errors; typed private parser causes stay intact.
    #[error("Saved chat validation failed: {0}")]
    Validation(#[source] nocterm_core::SharedCause),
    #[error("Saved chat validation failed: {0}")]
    ManagedValidation(#[source] crate::model::failure::ManagedNativeFailure),
    #[error("Could not encode or decode the saved chat: {0}")]
    ManagedJson(#[source] crate::model::failure::ManagedNativeFailure),
    #[error("retained history ownership failed: {0}")]
    Retention(#[source] nocterm_core::SharedError<crate::model::retention::RetentionError>),
    #[error("Saved chat contains an unsupported semantic record: {0}")]
    Model(#[from] crate::model::diagnostic::ModelError),
    #[error("Unrecognized saved chat document format")]
    FormatMismatch,
    #[error("Saved chat candidate does not match its prepared format context")]
    CandidateMismatch,
    #[error("Legacy history cannot represent {0:?}")]
    Compatibility(super::CompatibilityReason),
    #[error("Saved chat measurement arithmetic overflow")]
    ArithmeticOverflow,
    #[error("Invalid chat identity: {0}")]
    InvalidId(String),
    #[error("Saved chat identity does not match its requested identity")]
    IdentityMismatch,
    #[error("Unsupported saved chat version: {0}")]
    UnsupportedVersion(u32),
    #[error("This chat exceeds the saved collection limit.")]
    CollectionLimit,
    #[error("This chat exceeds its storage limit. Remove images or shorten the message.")]
    EncodedLimit,
    #[error(
        "This chat exceeds the structural memory limit. Shorten the message or remove attachments."
    )]
    StructuralLimit,
    #[error("An archived draft patch requires the shared document writer.")]
    ArchivedPatch,
    #[error("Could not encode or decode the saved chat: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
}
impl From<serde_json::Error> for HistoryCodecError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(SharedError::from_error(error))
    }
}

impl From<super::MeasurementError> for HistoryCodecError {
    fn from(error: super::MeasurementError) -> Self {
        match error {
            super::MeasurementError::Retention(error) => {
                Self::Retention(SharedError::from_error(error))
            }
            super::MeasurementError::Json(error) => Self::from(error),
            super::MeasurementError::Formatting(error) => Self::Formatting(error),
            super::MeasurementError::ArithmeticOverflow => Self::ArithmeticOverflow,
        }
    }
}

impl From<crate::model::json::deferred::JsonDecodeError> for HistoryCodecError {
    fn from(error: crate::model::json::deferred::JsonDecodeError) -> Self {
        use crate::model::json::deferred::JsonDecodeError;
        match error {
            JsonDecodeError::Limit { .. } => Self::StructuralLimit,
            JsonDecodeError::Arithmetic => Self::ArithmeticOverflow,
            JsonDecodeError::Json(error) => Self::from(error),
            error => Self::Validation(nocterm_core::SharedCause::new(error)),
        }
    }
}

#[cfg(test)]
mod tests;
