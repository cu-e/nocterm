//! A chat as the agent-chat renderer and composer see it. Borrowed presentation; every action
//! goes to the chat's own conversation.
use crate::{CloudChat, CloudChatError, SubmittedKey};
use gpui::{App, Entity, Subscription, Task, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationActions, ConversationError, ConversationSource, ConversationSubmission,
    ConversationSubmissionStart, ConversationToolApproval, ToolApprovalChoice, ToolApprovalView,
};
use nocterm_ai::conversation::{
    ComposerCapabilities, ConversationAction, ConversationPhase, ConversationView, PendingPromptKey,
};
use nocterm_ai::tool_display::ToolDisplay;
use nocterm_cloud_conversation_model::{
    CommandState, ConversationAction as CloudAction, IntentKey,
};
use std::rc::Rc;

#[derive(Clone)]
pub struct CloudConversationView(WeakEntity<CloudChat>);
impl CloudConversationView {
    pub fn new(chat: &Entity<CloudChat>) -> Self {
        Self(chat.downgrade())
    }
    fn chat(&self) -> Result<Entity<CloudChat>, ConversationError> {
        self.0.upgrade().ok_or(ConversationError::Stale)
    }
}
impl ConversationSource for CloudConversationView {
    fn identity(&self) -> gpui::EntityId {
        self.0.entity_id()
    }
    fn tool_approvals(
        &self,
        cx: &mut App,
    ) -> Result<Vec<Rc<dyn ConversationToolApproval>>, ConversationError> {
        let chat = self.chat()?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        let control = chat.transcript().control_revision();
        Ok(chat
            .transcript()
            .approvals()
            .iter()
            .map(|approval| {
                Rc::new(CloudApproval {
                    chat: self.0.clone(),
                    intent: approval.intent,
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
        let chat = self.chat()?;
        let chat = chat.read(cx);
        chat.check_current()
            .map_err(CloudChatError::into_conversation)?;
        let pending_prompts = chat.pending_rows();
        let phase = crate::thread::phase(chat.transcript());
        visitor(ConversationView {
            transcript: chat.thread(),
            phase,
            generating: phase == ConversationPhase::Running,
            accepts_updates: true,
            can_fork: false,
            can_prompt: chat.can_prompt(),
            can_allow: chat.allowable_run().is_some(),
            can_stop: chat.stoppable_run().is_some(),
            updated_at: chat.updated_at(),
            pending_commands: chat.pending_commands(),
            reconnecting: chat.reconnecting(),
            freshness: chat.shown_freshness(),
            // The runner owns its terminals, model and usage; a Cloud prompt is text.
            composer: ComposerCapabilities::default(),
            pending_prompts: &pending_prompts,
        });
        Ok(())
    }
    fn observe(
        &self,
        cx: &mut App,
        changed: Rc<dyn Fn(&mut App)>,
    ) -> Result<Subscription, ConversationError> {
        let chat = self.chat()?;
        Ok(cx.observe(&chat, move |_, cx| changed(cx)))
    }
    fn set_shown(&self, shown: bool, cx: &mut App) -> Result<(), ConversationError> {
        self.chat()?
            .update(cx, |chat, cx| chat.set_shown(shown, cx))
            .map_err(CloudChatError::into_conversation)
    }
    fn capture_actions(&self, cx: &App) -> Result<Rc<dyn ConversationActions>, ConversationError> {
        self.chat()?
            .read(cx)
            .check_current()
            .map_err(CloudChatError::into_conversation)?;
        let chat = self.chat()?;
        let chat = chat.read(cx);
        Ok(Rc::new(CloudActions {
            chat: self.0.clone(),
            shown: chat.transcript().shown_run().map(|run| run.run),
        }))
    }
}

/// Actions captured with the run the chat showed: a Stop or Allow drawn for one run never acts
/// on a run that replaced it.
struct CloudActions {
    chat: WeakEntity<CloudChat>,
    shown: Option<nocterm_cloud_conversation_model::RunKey>,
}
impl CloudActions {
    /// The Cloud command of `action`, against what the chat showed when it was captured.
    fn command(
        &self,
        action: ConversationAction,
        chat: &CloudChat,
    ) -> Result<CloudAction, ConversationError> {
        let shown = chat.transcript().shown_run().map(|run| run.run);
        let run_action = matches!(action, ConversationAction::Stop | ConversationAction::Allow);
        if run_action && shown != self.shown {
            return Err(ConversationError::Stale);
        }
        match action {
            ConversationAction::Prompt(text) if chat.can_prompt() => {
                Ok(CloudAction::Prompt { text })
            }
            ConversationAction::Stop => Ok(CloudAction::Stop {
                run: Some(chat.stoppable_run().ok_or(ConversationError::Unavailable)?),
            }),
            ConversationAction::Allow => Ok(CloudAction::Allow {
                run: chat.allowable_run().ok_or(ConversationError::Unavailable)?,
            }),
            ConversationAction::Prompt(_) => Err(ConversationError::Unavailable),
        }
    }
    fn send(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Result<(SubmittedKey, Task<Result<(), ConversationError>>), ConversationError> {
        let chat = self.chat.upgrade().ok_or(ConversationError::Stale)?;
        chat.update(cx, |chat, cx| {
            chat.check_current()
                .map_err(CloudChatError::into_conversation)?;
            let command = self.command(action, chat)?;
            Ok(chat.send(command, cx))
        })
    }
}
impl ConversationActions for CloudActions {
    fn submit(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Result<ConversationSubmissionStart, ConversationError> {
        let prompt = matches!(action, ConversationAction::Prompt(_));
        let (key, completion) = self.send(action, cx)?;
        Ok((
            Rc::new(CloudSubmission {
                chat: self.chat.clone(),
                key,
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
        match self.send(action, cx) {
            Ok((_, completion)) => completion,
            Err(error) => Task::ready(Err(error)),
        }
    }
    fn retry_prompt(
        &self,
        prompt: PendingPromptKey,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        chat.update(cx, |chat, cx| chat.retry_prompt(prompt, cx))
    }
    fn dismiss_prompt(
        &self,
        prompt: PendingPromptKey,
        cx: &mut App,
    ) -> Result<(), ConversationError> {
        let chat = self.chat.upgrade().ok_or(ConversationError::Stale)?;
        chat.update(cx, |chat, cx| chat.dismiss_prompt(prompt, cx))
    }
    fn wake(&self, cx: &mut App) -> Task<Result<(), ConversationError>> {
        let Some(chat) = self.chat.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        chat.update(cx, |chat, cx| chat.wake(cx))
    }
}

struct CloudSubmission {
    chat: WeakEntity<CloudChat>,
    key: SubmittedKey,
    prompt: bool,
}
impl CloudSubmission {
    fn state(&self, cx: &App) -> Option<CommandState> {
        let key = self.key.get()?;
        let chat = self.chat.upgrade()?;
        let chat = chat.read(cx);
        chat.transcript()
            .pending_command(key)
            .map(|known| known.state)
    }
}
impl ConversationSubmission for CloudSubmission {
    fn retry(&self, cx: &mut App) -> Task<Result<(), ConversationError>> {
        // The core already resends the same command until its window ends; what is left is
        // to say how it went.
        Task::ready(match self.state(cx) {
            Some(CommandState::Refused) => Err(ConversationError::Rejected),
            Some(CommandState::Unreachable) => Err(ConversationError::Unreachable),
            _ => Ok(()),
        })
    }
    /// Refused or unanswered for good, or no longer held because its conversation ended.
    fn refused(&self, cx: &App) -> bool {
        if self.chat.upgrade().is_none() {
            return true;
        }
        if self.key.get().is_none() {
            return false;
        }
        match self.state(cx) {
            Some(CommandState::Refused | CommandState::Unreachable) | None => true,
            Some(_) => false,
        }
    }
    fn pending_prompt(&self) -> Option<PendingPromptKey> {
        self.prompt
            .then(|| self.key.get().map(crate::pending::prompt_key))
            .flatten()
    }
}

struct CloudApproval {
    chat: WeakEntity<CloudChat>,
    intent: IntentKey,
    control: u64,
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
        let transcript = chat.transcript();
        if transcript.control_revision() != self.control {
            return Err(ConversationError::Unavailable);
        }
        let approval = transcript
            .approval(self.intent)
            .ok_or(ConversationError::Unavailable)?;
        let tool = transcript
            .tool(self.intent)
            .ok_or(ConversationError::Unavailable)?;
        let mut display = ToolDisplay::requested(
            tool.tool.clone(),
            serde_json::from_str(&tool.arguments_json).unwrap_or_default(),
        );
        display.destination.clone_from(&tool.destination);
        display.redacted = tool.redacted;
        let call = display
            .display_request()
            .ok_or(ConversationError::Unavailable)?;
        visitor(
            ToolApprovalView::new(&call, tool.destination.as_deref().unwrap_or(""), false)
                .with_display(&display)
                .with_expiry(Some(approval.expires_at_ms)),
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
        let approve = match choice {
            ToolApprovalChoice::Deny => false,
            ToolApprovalChoice::AllowOnce => true,
            ToolApprovalChoice::AllowForSession => {
                return Task::ready(Err(ConversationError::Unavailable));
            }
        };
        let intent = self.intent;
        let control = self.control;
        chat.update(cx, |chat, cx| {
            if chat.transcript().control_revision() != control
                || chat.transcript().approval(intent).is_none()
            {
                return Task::ready(Err(ConversationError::Unavailable));
            }
            chat.send(CloudAction::Answer { intent, approve }, cx).1
        })
    }
}
