//! Typed prompt admission refusals shared with panel presentation.
use super::super::document::DocumentError;
use nocterm_ai::history::{live::LiveError, physical::PhysicalError};
/// Why a message was not queued; shown in the composer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SubmitError {
    #[error("Wait for the saved chat to finish loading.")]
    Loading,
    #[error("This message was already sent.")]
    AlreadySent,
    #[error("Too many queued messages, images or attachments in this chat.")]
    TooMany,
    #[error(transparent)]
    Document(DocumentError),
}
impl From<DocumentError> for SubmitError {
    fn from(error: DocumentError) -> Self {
        match error {
            DocumentError::Live(LiveError::Physical(PhysicalError::QueuedPromptMissing)) => {
                Self::AlreadySent
            }
            DocumentError::Live(LiveError::Physical(PhysicalError::QueueLimit)) => Self::TooMany,
            other => Self::Document(other),
        }
    }
}
