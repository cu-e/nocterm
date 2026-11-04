//! Pending identities and durable bodies outlive an individual action waiter.
use futures::channel::oneshot;
use gpui::Task;
use nocterm_ai::conversation::{PendingPrompt, PendingPromptKey, PendingPromptState};
use nocterm_cloud_model::projection::ConversationIntent;
use nocterm_cloud_model::{
    identity::CommandId,
    projection::{MAX_PENDING_COMMANDS, ProjectionError, RecordKind, ports::ConversationCommand},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingCommandState {
    Staging,
    Pending,
    Delivered,
    Accepted,
    Rejected,
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCommandView {
    id: CommandId,
    state: PendingCommandState,
}
impl PendingCommandView {
    pub fn id(&self) -> CommandId {
        self.id
    }
    pub fn state(&self) -> PendingCommandState {
        self.state
    }
}

/// Why the waiter of one command learned that it did not succeed.
#[derive(Clone, Debug)]
pub(crate) enum CommandFailure {
    /// The source refused to stage, deliver or query it; its exact ID stays for a retry.
    Source(ProjectionError),
    /// The hub or the runner did not answer its delivery; its exact ID stays for a retry.
    Unreachable,
    /// The runner refused it for good.
    Rejected,
}
pub(crate) type CommandWaiter = oneshot::Sender<Result<(), CommandFailure>>;
pub(crate) type RetainedCommand = Allocated<ConversationCommand>;
pub(crate) type SubmissionReceipt = Rc<Cell<Option<bool>>>;

struct PendingCommand {
    retained: Arc<RetainedCommand>,
    state: PendingCommandState,
    operation: Option<Task<()>>,
    waiter: Option<CommandWaiter>,
    receipt: SubmissionReceipt,
    /// When it was sent, relative to the other commands and refused prompts.
    order: u64,
}
impl PendingCommand {
    fn prompt_state(&self) -> PendingPromptState {
        if self.operation.is_some() || self.waiter.is_some() {
            return PendingPromptState::Sending;
        }
        match self.state {
            PendingCommandState::Staging => PendingPromptState::Sending,
            PendingCommandState::Pending
            | PendingCommandState::Delivered
            | PendingCommandState::Accepted => PendingPromptState::Delivered,
            PendingCommandState::OutcomeUnknown => PendingPromptState::Unreachable,
            PendingCommandState::Rejected => PendingPromptState::Refused,
        }
    }
}
/// At most this many refused prompts stay shown; older ones are forgotten.
const MAX_REFUSED_PROMPTS: usize = 4;
/// A prompt the runner refused. Its command and payload retire with the refusal; only its
/// text stays, under a bytes reservation, until it is sent again or dismissed.
pub(crate) struct Refusal {
    id: CommandId,
    text: String,
    order: u64,
}
impl Refusal {
    pub(crate) fn bytes(&self) -> usize {
        self.text.len()
    }
}
struct RefusedPrompt {
    refusal: Refusal,
    _allocation: AllocationLease,
}
#[derive(Default)]
pub(crate) struct PendingCommands {
    commands: BTreeMap<CommandId, PendingCommand>,
    refused: VecDeque<RefusedPrompt>,
    sent: u64,
}
pub(crate) fn prompt_key(id: CommandId) -> PendingPromptKey {
    PendingPromptKey::from_bytes(*id.as_bytes())
}
fn prompt_text(command: &RetainedCommand) -> Option<&str> {
    match &command.value().action {
        ConversationIntent::Prompt(text) => Some(text),
        _ => None,
    }
}
impl PendingCommands {
    /// The prompts the runner has not accepted, in the order they were sent.
    pub(crate) fn prompts(&self) -> Vec<PendingPrompt<'_>> {
        let mut prompts: Vec<(u64, PendingPrompt<'_>)> = self
            .commands
            .iter()
            .filter_map(|(id, entry)| {
                let text = prompt_text(&entry.retained)?;
                Some((
                    entry.order,
                    PendingPrompt {
                        key: prompt_key(*id),
                        text,
                        state: entry.prompt_state(),
                    },
                ))
            })
            .chain(self.refused.iter().map(|refused| {
                (
                    refused.refusal.order,
                    PendingPrompt {
                        key: prompt_key(refused.refusal.id),
                        text: &refused.refusal.text,
                        state: PendingPromptState::Refused,
                    },
                )
            }))
            .collect();
        prompts.sort_by_key(|(order, _)| *order);
        prompts.into_iter().map(|(_, prompt)| prompt).collect()
    }
    /// The text of a refused prompt.
    pub(crate) fn refused_text(&self, id: CommandId) -> Option<&str> {
        self.refused
            .iter()
            .find(|refused| refused.refusal.id == id)
            .map(|refused| refused.refusal.text.as_str())
    }
    /// Removes a refused prompt; whether there was one.
    pub(crate) fn forget_refused(&mut self, id: CommandId) -> bool {
        let before = self.refused.len();
        self.refused.retain(|refused| refused.refusal.id != id);
        self.refused.len() != before
    }
    /// Shows a refused prompt, within the bound, for as long as its reservation lives.
    pub(crate) fn keep_refused(&mut self, refusal: Refusal, allocation: AllocationLease) {
        if self.refused.len() >= MAX_REFUSED_PROMPTS {
            self.refused.pop_front();
        }
        self.refused.push_back(RefusedPrompt {
            refusal,
            _allocation: allocation,
        });
    }
    pub(crate) fn views(&self) -> Vec<PendingCommandView> {
        self.commands
            .iter()
            .map(|(id, entry)| PendingCommandView {
                id: *id,
                state: entry.state,
            })
            .collect()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
    pub(crate) fn contains(&self, id: CommandId) -> bool {
        self.commands.contains_key(&id)
    }
    pub(crate) fn require_room(&self) -> Result<(), ProjectionError> {
        (self.commands.len() < MAX_PENDING_COMMANDS)
            .then_some(())
            .ok_or(ProjectionError::Capacity)
    }
    pub(crate) fn insert(
        &mut self,
        command: ConversationCommand,
        allocation: AllocationLease,
        waiter: CommandWaiter,
    ) -> Result<(), ProjectionError> {
        self.require_room()?;
        if self.commands.contains_key(&command.id) {
            return Err(ProjectionError::InvalidRecord(RecordKind::PendingCommand));
        }
        self.sent = self.sent.saturating_add(1);
        self.commands.insert(
            command.id,
            PendingCommand {
                retained: Arc::new(Allocated::new(command, allocation)),
                state: PendingCommandState::Staging,
                operation: None,
                waiter: Some(waiter),
                receipt: Rc::new(Cell::new(None)),
                order: self.sent,
            },
        );
        Ok(())
    }
    pub(crate) fn retry(
        &mut self,
        id: CommandId,
        waiter: CommandWaiter,
    ) -> Result<Arc<RetainedCommand>, ProjectionError> {
        let entry = self
            .commands
            .get_mut(&id)
            .ok_or(ProjectionError::InvalidRecord(RecordKind::PendingCommand))?;
        if entry.operation.is_some() || entry.waiter.is_some() {
            return Err(ProjectionError::Unavailable);
        }
        if matches!(
            entry.state,
            PendingCommandState::Accepted | PendingCommandState::Rejected
        ) {
            return Err(ProjectionError::Unavailable);
        }
        entry.waiter = Some(waiter);
        Ok(entry.retained.clone())
    }
    pub(crate) fn has_approval(&self, key: nocterm_cloud_model::projection::ApprovalKey) -> bool {
        self.commands.values().any(|entry| matches!(&entry.retained.value().action, ConversationIntent::AnswerApproval(request) if key.matches_request(request)))
    }
    pub(crate) fn item(&self, id: CommandId) -> Result<Arc<RetainedCommand>, ProjectionError> {
        self.commands
            .get(&id)
            .map(|entry| entry.retained.clone())
            .ok_or(ProjectionError::InvalidRecord(RecordKind::PendingCommand))
    }
    pub(crate) fn receipt(&self, id: CommandId) -> Result<SubmissionReceipt, ProjectionError> {
        self.commands
            .get(&id)
            .map(|entry| entry.receipt.clone())
            .ok_or(ProjectionError::InvalidRecord(RecordKind::PendingCommand))
    }
    pub(crate) fn staged(&mut self, id: CommandId) {
        if let Some(entry) = self.commands.get_mut(&id)
            && !matches!(
                entry.state,
                PendingCommandState::Delivered
                    | PendingCommandState::Accepted
                    | PendingCommandState::Rejected
            )
        {
            entry.state = PendingCommandState::Pending;
        }
    }
    pub(crate) fn attach(&mut self, id: CommandId, task: Task<()>) {
        if let Some(entry) = self.commands.get_mut(&id) {
            entry.operation = Some(task);
        }
    }
    /// Records the outcome of a delivery; a refused prompt's command retires and its text is
    /// returned for the owner to keep under a reservation.
    pub(crate) fn finish(
        &mut self,
        id: CommandId,
        result: Result<PendingCommandState, ProjectionError>,
    ) -> Option<Refusal> {
        let rejected = matches!(result, Ok(PendingCommandState::Rejected));
        if let Some(entry) = self.commands.get_mut(&id) {
            entry.operation.take();
            let observed = result
                .as_ref()
                .copied()
                .unwrap_or(PendingCommandState::OutcomeUnknown);
            match observed {
                PendingCommandState::Accepted => entry.receipt.set(Some(true)),
                PendingCommandState::Rejected => entry.receipt.set(Some(false)),
                _ => {}
            }
            // A later runner Unknown/error does not erase an already matched
            // durable storage ACK. Only authenticated terminal facts supersede it.
            if entry.state != PendingCommandState::Delivered
                || matches!(
                    observed,
                    PendingCommandState::Accepted | PendingCommandState::Rejected
                )
            {
                entry.state = observed;
            }
            if let Some(waiter) = entry.waiter.take() {
                let result = match result {
                    Ok(PendingCommandState::Rejected) => Err(CommandFailure::Rejected),
                    Ok(_) => Ok(()),
                    // Delivery results come from the transport: an unavailable one did not
                    // answer, which a retry can change.
                    Err(ProjectionError::Unavailable) => Err(CommandFailure::Unreachable),
                    Err(error) => Err(CommandFailure::Source(error)),
                };
                let _ = waiter.send(result);
            }
        }
        // A definitive refusal cannot produce a later accepted replay. Retire
        // its body after notifying the waiter; uncertain outcomes retain exactID.
        if !rejected {
            return None;
        }
        let entry = self.commands.remove(&id)?;
        Some(Refusal {
            id,
            text: prompt_text(&entry.retained)?.to_owned(),
            order: entry.order,
        })
    }
    /// Whether a pending inference grant already waits for the runner.
    pub(crate) fn has_inference_grant(&self) -> bool {
        self.commands.values().any(|entry| {
            matches!(
                entry.retained.value().action,
                ConversationIntent::GrantInference
            )
        })
    }
    /// Retires the accepted commands; reports whether one of them was a prompt.
    pub(crate) fn accepted(&mut self, accepted: &[CommandId]) -> bool {
        let mut prompt = false;
        for id in accepted {
            if let Some(mut entry) = self.commands.remove(id) {
                prompt |= prompt_text(&entry.retained).is_some();
                entry.receipt.set(Some(true));
                if let Some(waiter) = entry.waiter.take() {
                    let _ = waiter.send(Ok(()));
                }
            }
        }
        prompt
    }
    pub(crate) fn invalidate(&mut self) {
        self.refused.clear();
        for (_, mut entry) in std::mem::take(&mut self.commands) {
            if let Some(waiter) = entry.waiter.take() {
                let _ = waiter.send(Err(CommandFailure::Source(ProjectionError::Stale)));
            }
        }
    }
}
