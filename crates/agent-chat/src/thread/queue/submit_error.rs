//! Typed prompt admission refusals shared with panel presentation.
/// Why a message was not queued; shown in the composer.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("Wait for the saved chat to finish loading.")]
    Loading,
    #[error("This message was already sent.")]
    AlreadySent,
    #[error("Too many queued messages, images or attachments in this chat.")]
    TooMany,
    #[error(transparent)]
    Storage(#[from] nocterm_ai::history::HistoryCodecError),
}
