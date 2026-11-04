//! Pending identities and durable bodies outlive an individual action waiter.
use futures::channel::oneshot;
use gpui::Task;
use nocterm_cloud_model::{
    identity::CommandId,
    projection::{MAX_PENDING_COMMANDS, ProjectionError, RecordKind, ports::ConversationCommand},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::{cell::Cell, collections::BTreeMap, rc::Rc, sync::Arc};

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
}
#[derive(Default)]
pub(crate) struct PendingCommands(BTreeMap<CommandId, PendingCommand>);
impl PendingCommands {
    pub(crate) fn views(&self) -> Vec<PendingCommandView> {
        self.0
            .iter()
            .map(|(id, entry)| PendingCommandView {
                id: *id,
                state: entry.state,
            })
            .collect()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn contains(&self, id: CommandId) -> bool {
        self.0.contains_key(&id)
    }
    pub(crate) fn require_room(&self) -> Result<(), ProjectionError> {
        (self.0.len() < MAX_PENDING_COMMANDS)
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
        if self.0.contains_key(&command.id) {
            return Err(ProjectionError::InvalidRecord(RecordKind::PendingCommand));
        }
        self.0.insert(
            command.id,
            PendingCommand {
                retained: Arc::new(Allocated::new(command, allocation)),
                state: PendingCommandState::Staging,
                operation: None,
                waiter: Some(waiter),
                receipt: Rc::new(Cell::new(None)),
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
            .0
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
        self.0.values().any(|entry| matches!(&entry.retained.value().action,nocterm_cloud_model::projection::ConversationIntent::AnswerApproval(request) if key.matches_request(request)))
    }
    pub(crate) fn item(&self, id: CommandId) -> Result<Arc<RetainedCommand>, ProjectionError> {
        self.0
            .get(&id)
            .map(|entry| entry.retained.clone())
            .ok_or(ProjectionError::InvalidRecord(RecordKind::PendingCommand))
    }
    pub(crate) fn receipt(&self, id: CommandId) -> Result<SubmissionReceipt, ProjectionError> {
        self.0
            .get(&id)
            .map(|entry| entry.receipt.clone())
            .ok_or(ProjectionError::InvalidRecord(RecordKind::PendingCommand))
    }
    pub(crate) fn staged(&mut self, id: CommandId) {
        if let Some(entry) = self.0.get_mut(&id)
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
        if let Some(entry) = self.0.get_mut(&id) {
            entry.operation = Some(task);
        }
    }
    pub(crate) fn finish(
        &mut self,
        id: CommandId,
        result: Result<PendingCommandState, ProjectionError>,
    ) {
        let rejected = matches!(result, Ok(PendingCommandState::Rejected));
        if let Some(entry) = self.0.get_mut(&id) {
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
        if rejected {
            self.0.remove(&id);
        }
    }
    /// Whether a pending inference grant already waits for the runner.
    pub(crate) fn has_inference_grant(&self) -> bool {
        self.0.values().any(|entry| {
            matches!(
                entry.retained.value().action,
                nocterm_cloud_model::projection::ConversationIntent::GrantInference
            )
        })
    }
    /// Retires the accepted commands; reports whether one of them was a prompt.
    pub(crate) fn accepted(&mut self, accepted: &[CommandId]) -> bool {
        let mut prompt = false;
        for id in accepted {
            if let Some(mut entry) = self.0.remove(id) {
                prompt |= matches!(
                    entry.retained.value().action,
                    nocterm_cloud_model::projection::ConversationIntent::Prompt(_)
                );
                entry.receipt.set(Some(true));
                if let Some(waiter) = entry.waiter.take() {
                    let _ = waiter.send(Ok(()));
                }
            }
        }
        prompt
    }
    pub(crate) fn invalidate(&mut self) {
        for (_, mut entry) in std::mem::take(&mut self.0) {
            if let Some(waiter) = entry.waiter.take() {
                let _ = waiter.send(Err(CommandFailure::Source(ProjectionError::Stale)));
            }
        }
    }
}
