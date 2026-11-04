//! Borrowed presentation and actions retain the originally displayed authority.
use crate::{CloudChat, CloudChatError};
use gpui::{App, Entity, Subscription, Task, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationActions, ConversationError, ConversationSource, ConversationToolApproval,
    ToolApprovalChoice, ToolApprovalView,
};
use nocterm_ai::conversation::{
    ConversationAction, ConversationPhase, ConversationView, PendingPromptKey,
};
use nocterm_cloud_model::projection::ports::ConversationCoordinates;
use std::rc::Rc;

#[derive(Clone)]
pub struct CloudConversation(WeakEntity<CloudChat>);
impl CloudConversation {
    pub fn new(chat: &Entity<CloudChat>) -> Self {
        Self(chat.downgrade())
    }
}
impl ConversationSource for CloudConversation {
    fn identity(&self) -> gpui::EntityId {
        self.0.entity_id()
    }
    fn tool_approvals(
        &self,
        cx: &mut App,
    ) -> Result<Vec<Rc<dyn ConversationToolApproval>>, ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        let control = chat.projection().control();
        Ok(chat
            .projection()
            .approvals()
            .map(|card| {
                Rc::new(CloudApproval {
                    chat: self.0.clone(),
                    key: card.key(),
                    control,
                }) as Rc<dyn ConversationToolApproval>
            })
            .collect())
    }
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ConversationView<'a>),
    ) -> Result<(), ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        let pending_prompts = chat.pending.prompts();
        visitor(ConversationView {
            transcript: chat.projection().transcript(),
            phase: chat.projection().phase(),
            generating: chat.projection().phase() == ConversationPhase::Running,
            // check_current above fences the original replay producer, including
            // background observations after a run has stopped.
            accepts_updates: true,
            can_fork: false,
            can_prompt: chat.source.supports_prompt()
                && chat.projection().phase() != ConversationPhase::Deleted,
            can_allow: chat.can_allow(),
            can_stop: chat.projection().run().is_some()
                && !matches!(
                    chat.projection().phase(),
                    ConversationPhase::Deleted
                        | ConversationPhase::Stopped
                        | ConversationPhase::Completed
                ),
            updated_at: chat.projection().updated_at(),
            pending_commands: chat.pending_commands().len(),
            reconnecting: chat.reconnecting(),
            // The runner owns its terminals, model and usage; a Cloud prompt is text.
            composer: nocterm_ai::conversation::ComposerCapabilities::default(),
            pending_prompts: &pending_prompts,
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
    fn submit(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Result<nocterm_agent_chat::conversation::ConversationSubmissionStart, ConversationError>
    {
        let chat = self.chat.upgrade().ok_or(ConversationError::Stale)?;
        let prompt = matches!(action, ConversationAction::Prompt(_));
        let (id, receipt, completion) =
            chat.update(cx, |chat, cx| -> Result<_, ConversationError> {
                let (id, completion) = chat.submit(self.coordinates.clone(), action, cx)?;
                let receipt = chat
                    .pending
                    .receipt(id)
                    .map_err(|error| CloudChatError::from(error).into_conversation())?;
                Ok((id, receipt, completion))
            })?;
        Ok((
            Rc::new(CloudSubmission {
                chat: self.chat.clone(),
                id,
                receipt,
                prompt,
            }),
            completion,
        ))
    }
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
    fn retry_prompt(
        &self,
        prompt: PendingPromptKey,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        if let Err(error) = chat.read(cx).check_current() {
            return Task::ready(Err(error.into_conversation()));
        }
        chat.update(cx, |chat, cx| chat.retry_prompt(command(prompt), cx))
    }
    fn dismiss_prompt(
        &self,
        prompt: PendingPromptKey,
        cx: &mut App,
    ) -> Result<(), ConversationError> {
        let chat = self.chat.upgrade().ok_or(ConversationError::Stale)?;
        chat.update(cx, |chat, cx| chat.dismiss_prompt(command(prompt), cx))
    }
}
fn command(prompt: PendingPromptKey) -> nocterm_cloud_model::identity::CommandId {
    nocterm_cloud_model::identity::CommandId::from_bytes(*prompt.as_bytes())
}
struct CloudSubmission {
    chat: WeakEntity<CloudChat>,
    id: nocterm_cloud_model::identity::CommandId,
    receipt: super::pending::SubmissionReceipt,
    prompt: bool,
}
impl nocterm_agent_chat::conversation::ConversationSubmission for CloudSubmission {
    fn retry(&self, cx: &mut App) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        if let Err(error) = chat.read(cx).check_current() {
            return Task::ready(Err(error.into_conversation()));
        }
        match self.receipt.get() {
            Some(true) => return Task::ready(Ok(())),
            Some(false) => return Task::ready(Err(ConversationError::Rejected)),
            None => {}
        }
        chat.update(cx, |chat, cx| chat.retry(self.id, cx))
    }
    /// Refused by the runner, or no longer held because its source ended.
    fn refused(&self, cx: &App) -> bool {
        match self.receipt.get() {
            Some(accepted) => !accepted,
            None => self
                .chat
                .upgrade()
                .is_none_or(|chat| !chat.read(cx).pending.contains(self.id)),
        }
    }
    fn pending_prompt(&self) -> Option<PendingPromptKey> {
        self.prompt.then(|| super::pending::prompt_key(self.id))
    }
}

struct CloudApproval {
    chat: WeakEntity<CloudChat>,
    key: nocterm_cloud_model::projection::ApprovalKey,
    control: nocterm_cloud_model::identity::ChatControlRevision,
}
impl ConversationToolApproval for CloudApproval {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ToolApprovalView<'a>),
    ) -> Result<(), ConversationError> {
        let chat = self.chat.upgrade().ok_or(ConversationError::Stale)?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        if chat.projection().control() != self.control {
            return Err(ConversationError::Unavailable);
        }
        let card = chat
            .projection()
            .approval(self.key)
            .ok_or(ConversationError::Unavailable)?;
        let principal = card.intent().authority().session().target().principal();
        visitor(
            ToolApprovalView::new(
                card.call(),
                card.display().destination.as_deref().unwrap_or(""),
                false,
            )
            .with_display(card.display())
            .with_principal(principal.as_str())
            .with_expiry(Some(self.key.expires().as_millis())),
        );
        Ok(())
    }
    fn choose(
        &self,
        choice: ToolApprovalChoice,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        let choice = match choice {
            ToolApprovalChoice::Deny => nocterm_cloud_model::coordinator::ApprovalChoice::Deny,
            ToolApprovalChoice::AllowOnce => {
                nocterm_cloud_model::coordinator::ApprovalChoice::Approve
            }
            ToolApprovalChoice::AllowForSession => {
                return Task::ready(Err(ConversationError::Unavailable));
            }
        };
        chat.update(cx, |chat, cx| {
            chat.perform_approval(self.key, self.control, choice, cx)
        })
    }
}
