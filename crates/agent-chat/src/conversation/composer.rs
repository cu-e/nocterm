//! The prompt draft of a captured conversation: images and its next model. The source owns
//! admission and lifetime; a late picker or send completion cannot change a newer draft.
use super::ConversationError;
use gpui::{App, Task};
pub use nocterm_ai::conversation::{ConversationModel, ConversationProvider};
use nocterm_ai::images::PromptImage;

/// The non-text prompt draft of one conversation, retained by its source.
pub trait ConversationComposer {
    fn can_choose_provider(&self, _cx: &App) -> bool {
        false
    }
    fn providers(
        &self,
        _cx: &mut App,
    ) -> Task<Result<Vec<ConversationProvider>, ConversationError>> {
        Task::ready(Err(ConversationError::Unavailable))
    }
    fn select_provider(&self, _id: String, _cx: &mut App) -> Task<Result<(), ConversationError>> {
        Task::ready(Err(ConversationError::Unavailable))
    }
    /// Changes whenever the images, selected model or target server change.
    fn revision(&self, cx: &App) -> Result<u64, ConversationError>;
    /// Cheap immutable snapshots; image bytes are shared.
    fn images(&self, cx: &App) -> Vec<PromptImage>;
    /// Appends prepared images; rejects the entire batch if the combined draft exceeds its limits.
    fn accept_images(
        &self,
        images: Vec<PromptImage>,
        cx: &mut App,
    ) -> Result<(), ConversationError>;
    fn remove_image(&self, index: usize, cx: &mut App) -> Result<(), ConversationError>;
    /// A successful send clears only the draft it submitted.
    fn clear_images(&self, expected_revision: u64, cx: &mut App) -> Result<(), ConversationError>;
    fn models(&self, cx: &App) -> Vec<ConversationModel>;
    fn selected_model(&self, cx: &App) -> Option<ConversationModel>;
    fn select_model(&self, model: ConversationModel, cx: &mut App)
    -> Result<(), ConversationError>;
}
