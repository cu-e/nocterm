//! The renderer's borrowed source and captured action ports. No view or wire types.
use gpui::{App, EntityId, Subscription, Task};
use nocterm_ai::conversation::{ConversationAction, ConversationView, PendingPromptKey};
use std::rc::Rc;

mod approval;
pub use approval::{ConversationToolApproval, ToolApprovalChoice, ToolApprovalView};
mod local;
pub use local::LocalConversation;

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("The original conversation source is unavailable.")]
    Stale,
    #[error("This conversation cannot accept that action.")]
    Unavailable,
    /// The conversation's host could not be reached; the same action can be sent again.
    #[error("The agent cannot be reached right now. Send again to retry.")]
    Unreachable,
    /// The host refused this exact request; sending it again changes nothing.
    #[error("The agent refused this request.")]
    Rejected,
    #[error("The prompt exceeds the conversation history limits.")]
    Capacity,
    #[error("The conversation request has an unknown outcome; retry its original ID.")]
    OutcomeUnknown,
    #[error(transparent)]
    History(#[from] nocterm_ai::history::HistoryCodecError),
}

/// One admitted action, whose retries preserve its original identity and body.
pub trait ConversationSubmission {
    fn retry(&self, cx: &mut App) -> Task<Result<(), ConversationError>>;
    /// Whether the host refused the action for good, or no longer holds it, so retrying it can
    /// change nothing and sending the same input again is a new action. An accepted action is
    /// not refused: retrying it reports that it succeeded.
    fn refused(&self, cx: &App) -> bool;
    /// The pending prompt this submission shows in its conversation, when it shows one.
    fn pending_prompt(&self) -> Option<PendingPromptKey> {
        None
    }
}
pub type ConversationSubmissionStart = (
    Rc<dyn ConversationSubmission>,
    Task<Result<(), ConversationError>>,
);
/// Returned actions retain the original source and displayed control revision.
pub trait ConversationActions {
    fn submit(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Result<ConversationSubmissionStart, ConversationError>;
    fn perform(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>>;
    /// Sends a pending prompt that failed again: an unanswered one under its own identity, a
    /// refused one as a new prompt with the same text. A source without pending prompts has
    /// none to retry.
    fn retry_prompt(
        &self,
        _prompt: PendingPromptKey,
        _cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        Task::ready(Err(ConversationError::Unavailable))
    }
    /// Forgets a refused prompt the person no longer sends.
    fn dismiss_prompt(
        &self,
        _prompt: PendingPromptKey,
        _cx: &mut App,
    ) -> Result<(), ConversationError> {
        Err(ConversationError::Unavailable)
    }
    /// Wakes the producer of an unconfirmed history (a sleeping Cloud seat) and reads again.
    /// A source whose producer cannot be woken has nothing to wake.
    fn wake(&self, _cx: &mut App) -> Task<Result<(), ConversationError>> {
        Task::ready(Err(ConversationError::Unavailable))
    }
}

/// Implementations borrow rows only during the visitor and retain weak entities.
/// Viewer visibility never grants a run lease or implies a remote Stop.
pub trait ConversationSource {
    /// Presentation identity only; it confers no native/remote authority.
    fn identity(&self) -> EntityId;
    fn tool_approvals(
        &self,
        cx: &mut App,
    ) -> Result<Vec<Rc<dyn ConversationToolApproval>>, ConversationError>;
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
