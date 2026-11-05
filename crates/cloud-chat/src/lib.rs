//! The desktop's Cloud chat: one GPUI entity per open chat over the `CloudConversation` port,
//! without views.
//!
//! Owns: one installation's bounded conversation registry and its independent window
//! observers ([`CloudChatInstallation`]); per chat, the neutral transcript kept current by the
//! port's deltas, the thread the renderer draws from it, the shown freshness, the pending-prompt
//! presentation (ADR 0134) and the chat's share of the Local installation's history admission.
//! Provides the agent-chat `ConversationSource` of a chat, the same one composer as Local chats
//! use (ADR 0134), with its `ConversationServers` (the binding, binding and unbinding, and the
//! server the next prompts run on, ADR 0147), and its source-owned `ConversationComposer` draft
//! (prepared images, selected model and revision, ADR 0149).
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
//! one stays until it is sent again or dismissed and at most four refused ones are shown.
//! Incoming growth is admitted before mutation, and row/JSON conversion is prepared in one
//! quota-retaining background job per reading before its generation-bound atomic acceptance;
//! viewer close never sends Stop; a chat whose account ended fails and keeps nothing.
mod admission;
mod composer;
mod conversation;
mod error;
mod incoming;
mod installation;
mod pending;
mod servers;
mod thread;
mod transcript_cost;
pub use conversation::CloudConversationView;
pub use error::CloudChatError;
pub use installation::CloudChatInstallation;

use admission::Residency;
use futures::StreamExt as _;
use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, Task};
use incoming::{AdmittedUpdate, PreparedUpdate};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_agent_runtime::history_admission::HistoryAdmissionHandle;
use nocterm_ai::conversation::{ConversationFreshness, PendingPromptKey};
use nocterm_ai::thread::ThreadState;
use nocterm_cloud_conversation_model::{
    CloudConversation, CloudConversationError, CloudTranscript, CommandKey, CommandKind,
    CommandState, ConversationAction, ConversationUpdate, Freshness, RunKey, RunStatus,
};
use nocterm_core::allocation::Allocated;
use pending::{CommandFailure, PendingPrompts};
use std::{cell::Cell, rc::Rc, sync::Arc};
use thread::ThreadMirror;
use transcript_cost::TranscriptCost;

/// The key a submission learns once the conversation tracked its command.
pub(crate) type SubmittedKey = Rc<Cell<Option<CommandKey>>>;

pub struct CloudChat {
    conversation: Arc<dyn CloudConversation>,
    draft: composer::PromptDraft,
    transcript: CloudTranscript,
    thread: ThreadMirror,
    transcript_cost: TranscriptCost,
    /// What the core last reported (`Opening` before its first round); `None` until the
    /// subscription delivered one.
    freshness: Option<Freshness>,
    prompts: PendingPrompts,
    residency: Residency,
    subscription: Option<Task<()>>,
    subscription_generation: u64,
    membership: Option<Task<()>>,
    error: Option<CloudChatError>,
    /// The last read round went unanswered; the next change clears it.
    reconnecting: bool,
    /// The bound server the person picked for the next prompts (ADR 0147).
    target: Option<nocterm_cloud_conversation_model::ServerKey>,
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
            draft: composer::PromptDraft::default(),
            transcript: CloudTranscript::new(),
            thread: ThreadMirror::default(),
            transcript_cost: TranscriptCost::default(),
            freshness: None,
            prompts: PendingPrompts::default(),
            residency: Residency::new(admission, payload),
            subscription: None,
            subscription_generation: 0,
            membership: None,
            error: None,
            reconnecting: false,
            target: None,
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
    fn reserve_draft(
        &mut self,
        incoming: usize,
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        self.residency
            .fit(
                self.transcript_cost
                    .bytes()
                    .saturating_add(self.draft.bytes())
                    .saturating_add(self.prompts.bytes())
                    .saturating_add(incoming),
                cx,
            )
            .map_err(|error| CloudChatError::from(error).into_conversation())
    }
    pub(crate) fn port(&self) -> &Arc<dyn CloudConversation> {
        &self.conversation
    }
    /// The server the next prompts run on: the one the person picked while a prompt may still
    /// name it, else none (the runner's default destination).
    pub(crate) fn prompt_target(&self) -> Option<nocterm_cloud_conversation_model::ServerKey> {
        self.target.filter(|target| {
            self.transcript
                .servers()
                .selectable()
                .any(|server| &server.server == target)
        })
    }
    /// Picks the server the next prompts run on; only a selectable bound server.
    pub(crate) fn set_target(
        &mut self,
        target: Option<nocterm_cloud_conversation_model::ServerKey>,
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        self.check_current()
            .map_err(CloudChatError::into_conversation)?;
        if let Some(target) = target
            && !self
                .transcript
                .servers()
                .selectable()
                .any(|server| server.server == target)
        {
            return Err(nocterm_agent_chat::conversation::ServerProblem::NotSelectable.into());
        }
        if self.target == target {
            return Ok(());
        }
        self.draft
            .advance_revision()
            .map_err(CloudChatError::into_conversation)?;
        self.target = target;
        cx.notify();
        Ok(())
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
            self.subscription_generation = self
                .subscription_generation
                .checked_add(1)
                .ok_or(CloudConversationError::Capacity)?;
            let generation = self.subscription_generation;
            self.subscription = Some(cx.spawn(async move |chat, cx| {
                let mut batch = 0;
                while let Some(update) = updates.next().await {
                    let admitted = chat.update(cx, |chat, cx| chat.admit(update, generation, cx));
                    let Ok(Ok(admitted)) = admitted else { return };
                    let prepared = admitted.prepare(cx.background_executor()).await;
                    let applied = chat.update(cx, |chat, cx| chat.apply(prepared, generation, cx));
                    if !matches!(applied, Ok(Ok(()))) {
                        return;
                    }
                    batch += 1;
                    if batch == 16 {
                        yield_update_batch().await;
                        batch = 0;
                    }
                }
                let _ = chat.update(cx, |chat, cx| chat.reading_ended(generation, cx));
            }));
        }
        Ok(())
    }
    fn reading_ended(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.subscription.is_none() || self.subscription_generation != generation {
            return;
        }
        if let Err(error) = self.check_current() {
            self.fail(error, cx);
            return;
        }
        self.subscription.take();
        self.prompts.reading_ended();
        self.error
            .get_or_insert_with(|| CloudConversationError::Ended.into());
        cx.notify();
    }
    /// Reserves a candidate before either authoritative state or its mirror changes.
    fn admit(
        &mut self,
        update: ConversationUpdate,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Result<AdmittedUpdate, CloudChatError> {
        self.require_reading(generation, cx)?;
        match update {
            ConversationUpdate::Transcript(delta) => {
                let cost = self.transcript_cost.prepare(&delta);
                if let Err(error) = self.residency.fit(
                    cost.bytes()
                        .saturating_add(self.draft.bytes())
                        .saturating_add(self.prompts.bytes()),
                    cx,
                ) {
                    self.fail(error.into(), cx);
                    return Err(CloudChatError::Closed);
                }
                let allocation = self.residency.retain_work()?;
                Ok(AdmittedUpdate::Transcript(Box::new(Allocated::new(
                    (delta, cost),
                    allocation,
                ))))
            }
            ConversationUpdate::ComposerChanged => Ok(AdmittedUpdate::ComposerChanged),
            ConversationUpdate::Freshness(value) => Ok(AdmittedUpdate::Freshness(value)),
            ConversationUpdate::Refused(error) => Ok(AdmittedUpdate::Refused(error)),
        }
    }
    fn require_reading(
        &mut self,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), CloudChatError> {
        if self.subscription.is_none() || self.subscription_generation != generation {
            return Err(CloudChatError::Closed);
        }
        if let Err(error) = self.check_current() {
            self.fail(error, cx);
            return Err(CloudChatError::Closed);
        }
        Ok(())
    }
    /// Accepts a prepared delta only for its original source and reading generation.
    fn apply(
        &mut self,
        update: PreparedUpdate,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), CloudChatError> {
        self.require_reading(generation, cx)?;
        match update {
            PreparedUpdate::Transcript(prepared) => {
                (*prepared).map(|prepared| {
                    let (delta, cost, rows) = prepared.into_parts();
                    self.transcript_cost.accept(&delta, cost);
                    self.thread.accept(&delta, rows);
                    self.transcript.apply(delta);
                    self.thread.sync_usage(&self.transcript);
                });
                self.prompts.follow(&self.transcript);
                self.reconnecting = false;
                self.error = None;
            }
            PreparedUpdate::ComposerChanged => {
                self.draft
                    .capabilities_changed(self.conversation.models())?;
            }
            PreparedUpdate::Freshness(freshness) => {
                self.freshness = Some(freshness);
                self.reconnecting = false;
            }
            PreparedUpdate::Refused(error) => {
                self.reconnecting = matches!(error, CloudConversationError::Unavailable);
                self.error = Some(error.into());
            }
        }
        if !self.is_shown() && !self.prompts.busy() && self.pending_commands() == 0 {
            self.subscription.take();
        }
        cx.notify();
        Ok(())
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
            // While the chat is opening nothing is claimed, so nothing alarming is shown.
            None | Some(Freshness::Opening { .. } | Freshness::Current { .. }) => {
                ConversationFreshness::Live
            }
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
        self.send_replacing(action, None, cx)
    }
    fn send_replacing(
        &mut self,
        action: ConversationAction,
        replaces: Option<CommandKey>,
        cx: &mut Context<Self>,
    ) -> (SubmittedKey, Task<Result<(), ConversationError>>) {
        let key: SubmittedKey = Rc::new(Cell::new(None));
        if let Err(error) = self.resume(cx) {
            return (key, Task::ready(Err(error.into_conversation())));
        }
        if action.prompt_text().is_some() {
            if !self.prompts.admits(&action, replaces) {
                return (key, Task::ready(Err(ConversationError::Capacity)));
            }
            if let Err(error) = self.reserve_draft(
                pending::prompt_bytes(&action)
                    .saturating_sub(self.prompts.replacement_bytes(replaces)),
                cx,
            ) {
                return (key, Task::ready(Err(error)));
            }
        }
        let placeholder = action
            .prompt_text()
            .map(|_| self.prompts.sending(&action, replaces));
        let delivery = self.conversation.deliver(action);
        let learned = key.clone();
        cx.notify();
        let task = cx.spawn(async move |chat, cx| {
            let result = delivery.await;
            let (waiter, settled) = oneshot::channel();
            chat.update(cx, |chat, cx| {
                if let Some(placeholder) = placeholder {
                    chat.prompts
                        .tracked(placeholder, result.as_ref().ok().copied());
                    chat.prompts.follow(&chat.transcript);
                }
                cx.notify();
                result.and_then(|command| {
                    chat.conversation.check_current()?;
                    learned.set(Some(command));
                    chat.prompts.wait(
                        command,
                        waiter,
                        &chat.transcript,
                        chat.subscription.is_some(),
                    );
                    Ok(())
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
        let Some(original) = self.prompts.body(command) else {
            return Task::ready(Err(ConversationError::OriginalPromptUnavailable));
        };
        self.send_replacing(original, Some(command), cx).1
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
            self.draft.clear();
            self.transcript = CloudTranscript::new();
            self.thread.clear();
            self.transcript_cost.clear();
            self.residency.evicted();
        }
        self.prompts.reading_ended();
        self.error = Some(error);
        self.subscription.take();
        cx.notify();
    }
    fn can_evict(&self) -> bool {
        !self.is_shown()
            && !self.prompts.busy()
            && !self.prompts.holds_retry_bodies()
            && self.pending_commands() == 0
            && self.draft.images().is_empty()
    }
    fn evict(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_evict() {
            return false;
        }
        self.subscription.take();
        self.draft.clear();
        self.prompts.invalidate();
        self.transcript = CloudTranscript::new();
        self.thread.clear();
        self.transcript_cost.clear();
        self.freshness = None;
        self.residency.evicted();
        cx.notify();
        true
    }
}

/// A ready subscription must still let input and painting run between finite batches.
async fn yield_update_batch() {
    let mut yielded = false;
    std::future::poll_fn(|cx| {
        if std::mem::replace(&mut yielded, true) {
            std::task::Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await;
}

#[cfg(test)]
mod tests;
