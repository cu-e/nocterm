//! The port through which the desktop's Cloud chat entity reads and steers one chat.
use crate::{
    CloudConversationError, CommandKey, ConversationAction, ConversationDelta, ConversationKey,
    Freshness,
};
use futures::future::BoxFuture;
use futures::stream::BoxStream;

/// A call of the port.
pub type ConversationFuture<T> = BoxFuture<'static, Result<T, CloudConversationError>>;
/// The updates of one subscription.
pub type ConversationUpdates = BoxStream<'static, ConversationUpdate>;

/// What a subscription reports, in order.
#[derive(Clone, Debug)]
pub enum ConversationUpdate {
    Transcript(ConversationDelta),
    Freshness(Freshness),
    /// A read round was refused; the verified prefix stays and a later round tries again.
    Refused(CloudConversationError),
}

/// One verified Cloud chat of the signed-in account, opened for a window. Holding it is the
/// only way to read or steer the chat; it is bound to the account it was opened for and ends
/// with it.
pub trait CloudConversation: Send + Sync {
    /// The chat's stable key.
    fn key(&self) -> &ConversationKey;

    /// Whether prompts can be sent: the chat knows the model they name.
    fn can_prompt(&self) -> bool;

    /// Whether a sleeping managed seat serves the chat, so [`Self::wake`] can start it.
    fn can_wake(&self) -> bool;

    /// Starts reading when nothing reads yet and returns the updates from now on: first the
    /// whole held transcript as a reset delta and its freshness, then every change. Reading
    /// stops when every subscription and every delivery have ended.
    fn subscribe(&self) -> Result<ConversationUpdates, CloudConversationError>;

    /// Builds the command against the verified transcript and sends it; the command shows as
    /// pending at once and its outcome arrives as a transcript change. Resolves with the
    /// command's key once it is tracked.
    fn deliver(&self, action: ConversationAction) -> ConversationFuture<CommandKey>;

    /// Asks the runner again for the chat's head and reads what is new.
    fn refresh(&self);

    /// Wakes the managed seat of the chat, then refreshes.
    fn wake(&self) -> ConversationFuture<()>;

    /// Fails with [`CloudConversationError::Stale`] once the account this was opened for ended.
    fn check_current(&self) -> Result<(), CloudConversationError>;

    /// Resolves when the account this was opened for ends; a later sign-in does not revive it.
    fn ended(&self) -> BoxFuture<'static, ()>;
}
