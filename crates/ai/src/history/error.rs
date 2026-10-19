//! Semantic failures of the bounded saved-chat format.
use std::sync::Arc;

#[derive(Clone, Debug, thiserror::Error)]
pub enum HistoryCodecError {
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
    Json(#[source] Arc<serde_json::Error>),
}
impl From<serde_json::Error> for HistoryCodecError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(Arc::new(error))
    }
}
