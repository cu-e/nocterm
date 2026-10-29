//! A weak Local adapter delegates to the existing SessionLease/document lifecycle.
use super::{ConversationActions, ConversationError, ConversationSource};
use crate::thread::AgentThread;
use gpui::{App, Entity, Subscription, Task, WeakEntity};
use nocterm_ai::conversation::{ConversationAction, ConversationPhase, ConversationView};
use std::rc::Rc;

pub struct LocalConversation(WeakEntity<AgentThread>);
impl LocalConversation {
    pub fn new(thread: &Entity<AgentThread>) -> Self {
        Self(thread.downgrade())
    }
}
impl ConversationSource for LocalConversation {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ConversationView<'a>),
    ) -> Result<(), ConversationError> {
        let thread = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let thread = thread.read(cx);
        visitor(ConversationView {
            transcript: thread.transcript(),
            generating: thread.session_generating(),
            accepts_updates: thread.session_accepts_updates(),
            can_fork: true,
            phase: if thread.session_generating() {
                ConversationPhase::Running
            } else if thread.has_pending_approvals() {
                ConversationPhase::WaitingApproval
            } else if thread.session_phase() == nocterm_ai::session::SessionPhase::Queued {
                ConversationPhase::Queued
            } else if thread.session_stopped() {
                ConversationPhase::Stopped
            } else {
                ConversationPhase::Idle
            },
            updated_at: thread.updated_at(),
            pending_commands: thread.queued_prompt_ids().len(),
        });
        Ok(())
    }
    fn observe(
        &self,
        cx: &mut App,
        changed: Rc<dyn Fn(&mut App)>,
    ) -> Result<Subscription, ConversationError> {
        let thread = self.0.upgrade().ok_or(ConversationError::Stale)?;
        Ok(cx.observe(&thread, move |_, cx| changed(cx)))
    }
    fn set_shown(&self, shown: bool, cx: &mut App) -> Result<(), ConversationError> {
        let thread = self.0.upgrade().ok_or(ConversationError::Stale)?;
        thread.update(cx, |thread, cx| thread.set_shown(shown, cx));
        Ok(())
    }
    fn capture_actions(&self, cx: &App) -> Result<Rc<dyn ConversationActions>, ConversationError> {
        let thread = self.0.upgrade().ok_or(ConversationError::Stale)?;
        Ok(Rc::new(LocalActions {
            thread: self.0.clone(),
            key: thread.read(cx).document().key().clone(),
        }))
    }
}
struct LocalActions {
    thread: WeakEntity<AgentThread>,
    key: nocterm_ai::execution_origin::ChatKey,
}
impl ConversationActions for LocalActions {
    fn perform(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(thread) = self.thread.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        Task::ready(thread.update(cx, |thread, cx| {
            if thread.document().key() != &self.key {
                return Err(ConversationError::Stale);
            }
            match action {
                ConversationAction::Prompt(text) => match thread.submit(text, None, cx) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(ConversationError::Unavailable),
                    Err(error) => Err(error.into_conversation()),
                },
                ConversationAction::Stop => {
                    thread.stop(cx);
                    Ok(())
                }
            }
        }))
    }
}
