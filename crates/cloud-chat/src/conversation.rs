//! Borrowed presentation and actions retain the originally displayed authority.
use crate::{CloudChat, CloudChatError};
use gpui::{App, Entity, Subscription, Task, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationActions, ConversationError, ConversationSource,
};
use nocterm_ai::conversation::{ConversationAction, ConversationPhase, ConversationView};
use nocterm_cloud_model::projection::ports::ConversationCoordinates;
use std::rc::Rc;

pub struct CloudConversation(WeakEntity<CloudChat>);
impl CloudConversation {
    pub fn new(chat: &Entity<CloudChat>) -> Self {
        Self(chat.downgrade())
    }
}
impl ConversationSource for CloudConversation {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ConversationView<'a>),
    ) -> Result<(), ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        visitor(ConversationView {
            transcript: chat.projection().transcript(),
            phase: chat.projection().phase(),
            generating: chat.projection().phase() == ConversationPhase::Running,
            // check_current above fences the original replay producer, including
            // background observations after a run has stopped.
            accepts_updates: true,
            can_fork: false,
            updated_at: chat.projection().updated_at(),
            pending_commands: chat.pending_commands().len(),
        });
        Ok(())
    }
    fn observe(
        &self,
        cx: &mut App,
        changed: Rc<dyn Fn(&mut App)>,
    ) -> Result<Subscription, ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        Ok(cx.observe(&chat, move |_, cx| changed(cx)))
    }
    fn set_shown(&self, shown: bool, cx: &mut App) -> Result<(), ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        chat.update(cx, |chat, cx| chat.set_shown(shown, cx))
            .map_err(CloudChatError::into_conversation)
    }
    fn capture_actions(&self, cx: &App) -> Result<Rc<dyn ConversationActions>, ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        Ok(Rc::new(CloudActions {
            chat: self.0.clone(),
            coordinates: chat.coordinates(),
        }))
    }
}
struct CloudActions {
    chat: WeakEntity<CloudChat>,
    coordinates: ConversationCoordinates,
}
impl ConversationActions for CloudActions {
    fn perform(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        chat.update(cx, |chat, cx| {
            chat.perform(self.coordinates.clone(), action, cx)
        })
    }
}
