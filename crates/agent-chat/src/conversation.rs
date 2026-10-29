//! The renderer's borrowed source and captured action ports. No view or wire types.
use gpui::{App, Subscription, Task};
use nocterm_ai::conversation::{ConversationAction, ConversationView};
use std::rc::Rc;

mod local;
pub use local::LocalConversation;

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("The original conversation source is unavailable.")]
    Stale,
    #[error("This conversation cannot accept that action.")]
    Unavailable,
    #[error("The prompt exceeds the conversation history limits.")]
    Capacity,
    #[error("The conversation request has an unknown outcome; retry its original ID.")]
    OutcomeUnknown,
    #[error(transparent)]
    History(#[from] nocterm_ai::history::HistoryCodecError),
}

/// Returned actions retain the original source and displayed control revision.
pub trait ConversationActions {
    fn perform(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>>;
}

/// Implementations borrow rows only during the visitor and retain weak entities.
/// Viewer visibility never grants a run lease or implies a remote Stop.
pub trait ConversationSource {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ConversationView<'a>),
    ) -> Result<(), ConversationError>;
    fn observe(
        &self,
        cx: &mut App,
        changed: Rc<dyn Fn(&mut App)>,
    ) -> Result<Subscription, ConversationError>;
    fn set_shown(&self, shown: bool, cx: &mut App) -> Result<(), ConversationError>;
    fn capture_actions(&self, cx: &App) -> Result<Rc<dyn ConversationActions>, ConversationError>;
}
