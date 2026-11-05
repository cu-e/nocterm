//! Why a conversation refused or failed.
use nocterm_core::error::SharedCause;

/// What the caller of [`crate::CloudConversation`] can react to. A cause from the device core
/// or the storage is kept as the source.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloudConversationError {
    /// The signed-in account or hub this conversation was opened for ended.
    #[error("the original Cloud account of this chat has ended")]
    Stale,
    /// The hub cannot be reached now; the same request can be made again.
    #[error("the Cloud hub cannot be reached right now")]
    Unavailable,
    /// The hub does not offer what this needs.
    #[error("this hub does not support that Cloud chat feature")]
    Unsupported,
    /// The transcript has no such run, or none to stop.
    #[error("the chat has no such run")]
    NoRun,
    /// No pending approval matches the answer.
    #[error("no pending approval matches this answer")]
    NoApproval,
    /// The chat has no model to prompt with.
    #[error("the chat has no model to prompt with")]
    NoModel,
    /// The chat was deleted.
    #[error("the chat was deleted")]
    Deleted,
    /// Too many commands or chats at once.
    #[error("too many Cloud commands are in use")]
    Capacity,
    /// The conversation stopped reading and accepts nothing more.
    #[error("the Cloud chat is closed")]
    Ended,
    /// The device core refused the chat's records or a command.
    #[error("the Cloud chat was refused")]
    Refused(#[source] SharedCause),
    /// Binding servers, or naming one in a prompt, was refused (ADR 0139).
    #[error("the chat's servers were refused: {0}")]
    Servers(crate::ServerRefusal),
    /// The device's private chat files failed.
    #[error("the Cloud chat files could not be used")]
    Storage(#[source] SharedCause),
}
impl CloudConversationError {
    /// Whether the same request may succeed later without anything changing.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Unavailable | Self::Capacity)
    }
}
