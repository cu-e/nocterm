//! A weak Local adapter delegates to the existing SessionLease/document lifecycle.
use super::{
    ConversationActions, ConversationError, ConversationSource, ConversationToolApproval,
    ToolApprovalChoice, ToolApprovalView,
};
use crate::thread::AgentThread;
use gpui::{App, Entity, EntityId, Subscription, Task, WeakEntity};
use nocterm_ai::conversation::{ConversationAction, ConversationPhase, ConversationView};
use std::rc::Rc;

pub struct LocalConversation(WeakEntity<AgentThread>);
impl LocalConversation {
    pub fn new(thread: &Entity<AgentThread>) -> Self {
        Self(thread.downgrade())
    }
}
impl ConversationSource for LocalConversation {
    fn identity(&self) -> EntityId {
        self.0.entity_id()
    }
    fn tool_approvals(
        &self,
        cx: &mut App,
    ) -> Result<Vec<Rc<dyn ConversationToolApproval>>, ConversationError> {
        let thread = self.0.upgrade().ok_or(ConversationError::Stale)?;
        let tokens = thread.read(cx).tool_approval_tokens();
        let key = thread.read(cx).document().key().clone();
        tokens
            .into_iter()
            .map(|token| {
                let (target, reusable) = thread.update(cx, |thread, cx| {
                    let call = thread
                        .tool_approval_call(token)
                        .cloned()
                        .ok_or(ConversationError::Unavailable)?;
                    Ok::<_, ConversationError>((
                        thread.describe_target(&call, cx),
                        !thread.unsafe_live_input(&call, cx),
                    ))
                })?;
                Ok(Rc::new(LocalApproval {
                    thread: self.0.clone(),
                    key: key.clone(),
                    token,
                    target,
                    reusable,
                }) as Rc<dyn ConversationToolApproval>)
            })
            .collect()
    }
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
            can_prompt: !thread.session_sign_in_required(),
            can_stop: thread.session_generating() || thread.queued_prompt_ids().len() != 0,
            can_allow: false,
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
            ticket: thread.read(cx).session_ticket(),
        }))
    }
}
struct LocalActions {
    thread: WeakEntity<AgentThread>,
    key: nocterm_ai::execution_origin::ChatKey,
    ticket: nocterm_ai::session::Ticket,
}
impl ConversationActions for LocalActions {
    fn submit(
        &self,
        action: ConversationAction,
        cx: &mut App,
    ) -> Result<super::ConversationSubmissionStart, ConversationError> {
        // ACP supplies no durable Cloud ID/query port. Preserve one actual
        // dispatch; an ambiguous retry cannot silently redispatch it.
        Ok((Rc::new(LocalSubmission), self.perform(action, cx)))
    }
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
                ConversationAction::Allow => Err(ConversationError::Unavailable),
                ConversationAction::Stop => {
                    if thread.session_ticket() != self.ticket {
                        return Err(ConversationError::Stale);
                    }
                    thread.stop(cx);
                    Ok(())
                }
            }
        }))
    }
}
struct LocalSubmission;
impl super::ConversationSubmission for LocalSubmission {
    fn retry(&self, _: &mut App) -> Task<Result<(), ConversationError>> {
        Task::ready(Err(ConversationError::Unavailable))
    }
}

struct LocalApproval {
    thread: WeakEntity<AgentThread>,
    key: nocterm_ai::execution_origin::ChatKey,
    token: crate::thread::ApprovalToken,
    target: String,
    reusable: bool,
}
impl ConversationToolApproval for LocalApproval {
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ToolApprovalView<'a>),
    ) -> Result<(), ConversationError> {
        let thread = self.thread.upgrade().ok_or(ConversationError::Stale)?;
        let thread = thread.read(cx);
        if thread.document().key() != &self.key {
            return Err(ConversationError::Stale);
        }
        let call = thread
            .tool_approval_call(self.token)
            .ok_or(ConversationError::Unavailable)?;
        visitor(ToolApprovalView::new(call, &self.target, self.reusable));
        Ok(())
    }
    fn choose(
        &self,
        choice: ToolApprovalChoice,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let Some(thread) = self.thread.upgrade() else {
            return Task::ready(Err(ConversationError::Stale));
        };
        let result = thread.update(cx, |thread, cx| {
            if thread.document().key() != &self.key {
                return Err(ConversationError::Stale);
            }
            if thread.tool_approval_call(self.token).is_none() {
                return Err(ConversationError::Unavailable);
            }
            if choice == ToolApprovalChoice::AllowForSession && !self.reusable {
                return Err(ConversationError::Unavailable);
            }
            thread.approve_tool(
                self.token,
                choice != ToolApprovalChoice::Deny,
                choice == ToolApprovalChoice::AllowForSession,
                cx,
            );
            Ok(())
        });
        Task::ready(result)
    }
}
