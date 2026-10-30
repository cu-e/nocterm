//! Narrow caller-owned handle for presenting an already captured conversation.
use gpui_kit::{AnyWindowHandle, App, SharedString, WeakEntity, Window};
use nocterm_agent_chat::conversation::{ConversationError, ConversationSource};
use std::rc::Rc;

#[derive(Clone)]
pub struct ConversationPanel {
    panel: WeakEntity<crate::panel::AgentPanel>,
    window: AnyWindowHandle,
}
impl ConversationPanel {
    pub(crate) fn new(
        panel: WeakEntity<crate::panel::AgentPanel>,
        window: AnyWindowHandle,
    ) -> Self {
        Self { panel, window }
    }
    /// The source factory supplies the original authenticated capture. Presenting
    /// it neither grants execution nor converts a remote chat into a Local chat.
    pub fn open(
        &self,
        source: Rc<dyn ConversationSource>,
        title: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ConversationError> {
        if self.window != window.window_handle() {
            return Err(ConversationError::Stale);
        }
        self.panel
            .update(cx, |panel, cx| {
                panel.open_conversation(source, title, window, cx)
            })
            .map_err(|_| ConversationError::Stale)?
    }
    /// Detaches this window; it never sends Stop to the producer.
    pub fn close(&self, cx: &mut App) -> Result<(), ConversationError> {
        self.panel
            .update(cx, |panel, cx| panel.close_conversation(cx))
            .map_err(|_| ConversationError::Stale)
    }
}
