//! The desktop's Cloud chat: one GPUI entity per open chat over the `CloudConversation` port,
//! without views.
//!
//! Owns: one installation's bounded conversation registry and its independent window
//! observers ([`CloudChatInstallation`]); per chat, the neutral transcript kept current by the
//! port's deltas, the thread the renderer draws from it, the shown freshness, the pending-prompt
//! presentation (ADR 0134) and the chat's share of the Local installation's history admission.
//! Provides the agent-chat `ConversationSource` of a chat, the same one composer as Local chats
//! use (ADR 0134).
//! Does not own: login/unlock, the device protocol, verification, the reduction of runner
//! events, checkpoints, command delivery and its retries (`nocterm-hub-io` runs the contract
//! device core behind the port), runner execution, views or Local SessionLease lifecycle.
//! Ports used: `nocterm-cloud-conversation-model::CloudConversation` and
//! agent-runtime::history_admission. Ports provided: agent-chat::ConversationSource.
//! Settings sections: none. Globals: none.
//! Invariants: the transcript changes only by the port's deltas, so it is always the verified
//! prefix the device core accepted; freshness is shown as the core reports it, and a chat
//! whose runner did not answer says so and offers Wake (sleeping seats); a prompt shows at
//! once and follows its command's state until the runner accepts it, a refused or unanswered
//! one stays until it is sent again or dismissed and at most four refused ones are shown;
//! viewer close never sends Stop; a chat whose account ended fails and keeps nothing.
mod admission;
mod conversation;
mod error;
mod installation;
mod pending;
mod thread;
pub use conversation::CloudConversationView;
pub use error::CloudChatError;
pub use installation::CloudChatInstallation;

use admission::Residency;
use futures::StreamExt as _;
use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, Task};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_agent_runtime::history_admission::HistoryAdmissionHandle;
use nocterm_ai::conversation::{ConversationFreshness, PendingPromptKey};
use nocterm_ai::thread::ThreadState;
use nocterm_cloud_conversation_model::{
    CloudConversation, CloudConversationError, CloudTranscript, CommandKey, CommandKind,
    CommandState, ConversationAction, ConversationUpdate, Freshness, RunKey, RunStatus,
};
use pending::{CommandFailure, PendingPrompts};
use std::{cell::Cell, rc::Rc, sync::Arc};
use thread::ThreadMirror;

/// The key a submission learns once the conversation tracked its command.
pub(crate) type SubmittedKey = Rc<Cell<Option<CommandKey>>>;

pub struct CloudChat {
    conversation: Arc<dyn CloudConversation>,
    transcript: CloudTranscript,
    thread: ThreadMirror,
    /// What the core reported after a round; `None` until the first round of a subscription.
    freshness: Option<Freshness>,
    prompts: PendingPrompts,
    residency: Residency,
    subscription: Option<Task<()>>,
    membership: Option<Task<()>>,
    error: Option<CloudChatError>,
    /// The last read round went unanswered; the next change clears it.
    reconnecting: bool,
}
impl CloudChat {
    /// Every partition receives the same handle from RuntimeHandle::history_admission.
    /// Construction does not create a new budget or an execution/session lease.
    pub fn open(
        conversation: Arc<dyn CloudConversation>,
        admission: HistoryAdmissionHandle,
        cx: &mut App,
    ) -> Result<Entity<Self>, CloudChatError> {
        conversation.check_current()?;
        let payload = admission.reserve_payload_reclaiming(None, cx)?;
        let entity = cx.new(|_| Self {
            conversation,
            transcript: CloudTranscript::new(),
            thread: ThreadMirror::default(),
            freshness: None,
            prompts: PendingPrompts::default(),
            residency: Residency::new(admission, payload),
            subscription: None,
            membership: None,
            error: None,
            reconnecting: false,
        });
        entity.update(cx, |chat, cx| {
            chat.residency.register(cx)?;
            let ended = chat.conversation.ended();
            chat.membership = Some(cx.spawn(async move |chat, cx| {
                ended.await;
                let _ = chat.update(cx, |chat, cx| {
                    chat.fail(CloudConversationError::Stale.into(), cx);
                });
            }));
            chat.resume(cx)
        })?;
        Ok(entity)
    }
    pub fn transcript(&self) -> &CloudTranscript {
        &self.transcript
    }
    /// The thread the renderer draws.
    pub fn thread(&self) -> &ThreadState {
        self.thread.thread()
    }
    /// How far the shown history is confirmed; `None` before the first read round ended.
    pub fn freshness(&self) -> Option<Freshness> {
        self.freshness
    }
    pub fn last_error(&self) -> Option<&CloudChatError> {
        self.error.as_ref()
    }
    pub fn is_shown(&self) -> bool {
        self.residency.shown()
    }
    pub fn conversation(
        entity: &Entity<Self>,
    ) -> Rc<dyn nocterm_agent_chat::conversation::ConversationSource> {
        Rc::new(CloudConversationView::new(entity))
    }
    pub fn check_current(&self) -> Result<(), CloudChatError> {
        self.conversation
            .check_current()
            .map_err(CloudChatError::from)
    }
    pub(crate) fn same_conversation(&self, other: &Arc<dyn CloudConversation>) -> bool {
        self.conversation.key() == other.key()
    }
    fn updated_at(&self) -> u64 {
        self.transcript.updated_at_ms() / 1000
    }
    fn resume(&mut self, cx: &mut Context<Self>) -> Result<(), CloudChatError> {
        self.check_current()?;
        if self.subscription.is_none() {
            let mut updates = self.conversation.subscribe()?;
            self.freshness = None;
            self.subscription = Some(cx.spawn(async move |chat, cx| {
                let mut first_freshness = true;
                while let Some(update) = updates.next().await {
                    let provisional =
                        matches!(update, ConversationUpdate::Freshness(_)) && first_freshness;
                    if matches!(update, ConversationUpdate::Freshness(_)) {
                        first_freshness = false;
                    }
                    let applied = chat.update(cx, |chat, cx| chat.apply(update, provisional, cx));
                    if !matches!(applied, Ok(Ok(()))) {
                        return;
                    }
                }
            }));
        }
        Ok(())
    }
    /// Applies one update of the subscription; an update that does not fit the admission
    /// ends the subscription.
    fn apply(
        &mut self,
        update: ConversationUpdate,
        provisional: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), CloudChatError> {
        match update {
            ConversationUpdate::Transcript(delta) => {
                let changed_from = if delta.reset || delta.first_index > self.first_index() {
                    Some(0)
                } else {
                    delta.changed_from
                };
                self.transcript.apply(delta);
                if let Err(error) = self
                    .residency
                    .fit(thread::resident_bytes(&self.transcript), cx)
                {
                    self.fail(error.into(), cx);
                    return Err(CloudChatError::Closed);
                }
                self.thread.follow(&self.transcript, changed_from);
                self.prompts.follow(&self.transcript);
                self.reconnecting = false;
                self.error = None;
            }
            ConversationUpdate::Freshness(freshness) => {
                if !provisional {
                    self.freshness = Some(freshness);
                    self.reconnecting = false;
                }
            }
            ConversationUpdate::Refused(error) => {
                self.reconnecting = matches!(error, CloudConversationError::Unavailable);
                self.error = Some(error.into());
            }
        }
        cx.notify();
        Ok(())
    }
    fn first_index(&self) -> u64 {
        self.transcript.rows().first().map_or(0, |row| row.index)
    }
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) -> Result<(), CloudChatError> {
        self.check_current()?;
        self.residency.set_shown(shown);
        if shown {
            self.resume(cx)?;
        } else if !self.prompts.busy() && self.pending_commands() == 0 {
            // Nobody looks and nothing waits for an answer: reading pauses until a window
            // shows the chat again, which reads on from the conversation's checkpoint.
            self.subscription.take();
        }
        cx.notify();
        Ok(())
    }
    /// The run the person may allow to use the model: theirs, waiting for its grant, and no
    /// grant on its way (ADR 0129).
    pub(crate) fn allowable_run(&self) -> Option<RunKey> {
        if !self.conversation.can_prompt() {
            return None;
        }
        let run = self.transcript.shown_run()?;
        let own = self
            .transcript
            .pending_command(run.prompt_command)
            .is_some_and(|known| known.kind == CommandKind::Prompt);
        let granting = self.transcript.pending().iter().any(|known| {
            known.kind == CommandKind::Allow
                && known.run == Some(run.run)
                && matches!(known.state, CommandState::Sending | CommandState::Delivered)
        });
        (own && !granting && matches!(run.status, RunStatus::Queued | RunStatus::NeedsGrant))
            .then_some(run.run)
    }
    pub(crate) fn stoppable_run(&self) -> Option<RunKey> {
        let run = self.transcript.shown_run()?;
        (!self.transcript.deleted() && !run.status.finished()).then_some(run.run)
    }
    pub(crate) fn can_prompt(&self) -> bool {
        self.conversation.can_prompt() && !self.transcript.deleted()
    }
    pub(crate) fn shown_freshness(&self) -> ConversationFreshness {
        match self.freshness {
            None | Some(Freshness::Current { .. }) => ConversationFreshness::Live,
            Some(Freshness::Unconfirmed { .. }) => ConversationFreshness::Unconfirmed {
                can_wake: self.conversation.can_wake(),
            },
            Some(Freshness::Withheld { .. }) => ConversationFreshness::Withheld,
        }
    }
    pub(crate) fn reconnecting(&self) -> bool {
        self.reconnecting && self.subscription.is_some()
    }
    pub(crate) fn pending_rows(&self) -> Vec<nocterm_ai::conversation::PendingPrompt<'_>> {
        self.prompts.rows(&self.transcript)
    }
    pub(crate) fn pending_commands(&self) -> usize {
        self.transcript
            .pending()
            .iter()
            .filter(|known| !known.state.settled())
            .count()
    }
    /// Sends `action`; the task resolves once the command was delivered or settled badly.
    pub(crate) fn send(
        &mut self,
        action: ConversationAction,
        cx: &mut Context<Self>,
    ) -> (SubmittedKey, Task<Result<(), ConversationError>>) {
        let key: SubmittedKey = Rc::new(Cell::new(None));
        if let Err(error) = self.check_current() {
            return (key, Task::ready(Err(error.into_conversation())));
        }
        let placeholder = action.prompt_text().map(|text| self.prompts.sending(text));
        let delivery = self.conversation.deliver(action);
        let learned = key.clone();
        cx.notify();
        let task = cx.spawn(async move |chat, cx| {
            let result = delivery.await;
            let (waiter, settled) = oneshot::channel();
            chat.update(cx, |chat, cx| {
                if let Some(placeholder) = placeholder {
                    chat.prompts.tracked(placeholder);
                }
                cx.notify();
                result.map(|command| {
                    learned.set(Some(command));
                    chat.prompts.wait(command, waiter, &chat.transcript);
                })
            })
            .map_err(|_| ConversationError::Stale)?
            .map_err(|error| CloudChatError::from(error).into_conversation())?;
            match settled.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(CommandFailure::Rejected)) => Err(ConversationError::Rejected),
                Ok(Err(CommandFailure::Unreachable)) => Err(ConversationError::Unreachable),
                Ok(Err(CommandFailure::Stale)) | Err(_) => Err(ConversationError::Stale),
            }
        });
        (key, task)
    }
    /// Sends a failed prompt's text again as a new prompt, which replaces its row.
    pub(crate) fn retry_prompt(
        &mut self,
        prompt: PendingPromptKey,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ConversationError>> {
        let command = pending::command_key(prompt);
        let Some(known) = self.transcript.pending_command(command) else {
            return Task::ready(Err(ConversationError::Unavailable));
        };
        if !matches!(
            known.state,
            CommandState::Refused | CommandState::Unreachable
        ) {
            return Task::ready(Ok(()));
        }
        let text = known.text.clone();
        self.prompts.dismiss(command, &self.transcript);
        self.send(ConversationAction::Prompt { text }, cx).1
    }
    pub(crate) fn dismiss_prompt(
        &mut self,
        prompt: PendingPromptKey,
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        if !self
            .prompts
            .dismiss(pending::command_key(prompt), &self.transcript)
        {
            return Err(ConversationError::Unavailable);
        }
        cx.notify();
        Ok(())
    }
    /// Wakes the chat's sleeping seat; the conversation reads again once it is up.
    pub(crate) fn wake(&mut self, cx: &mut Context<Self>) -> Task<Result<(), ConversationError>> {
        if !self.conversation.can_wake() {
            return Task::ready(Err(ConversationError::Unavailable));
        }
        let waking = self.conversation.wake();
        cx.spawn(async move |_, _| {
            waking
                .await
                .map_err(|error| CloudChatError::from(error).into_conversation())
        })
    }
    fn fail(&mut self, error: CloudChatError, cx: &mut Context<Self>) {
        if matches!(
            error,
            CloudChatError::Conversation(CloudConversationError::Stale)
        ) {
            self.prompts.invalidate();
            self.transcript = CloudTranscript::new();
            self.thread.clear();
            self.residency.evicted();
        }
        self.error = Some(error);
        self.subscription.take();
        cx.notify();
    }
    fn can_evict(&self) -> bool {
        !self.is_shown() && !self.prompts.busy() && self.pending_commands() == 0
    }
    fn evict(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_evict() {
            return false;
        }
        self.subscription.take();
        self.transcript = CloudTranscript::new();
        self.thread.clear();
        self.freshness = None;
        self.residency.evicted();
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests;
