//! Actor-owned byte reservations and bounded payload-free delivery futures.
use super::{format, frame::Frame};
use futures::{FutureExt, future::BoxFuture};
use nocterm_cloud_model::{
    coordinator::*,
    journal::{CommandReceipt, JournalError},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use std::sync::Arc;

const ACK_BYTES: usize = 16 * 1024;
#[derive(Clone)]
pub(super) struct Intake {
    budget: CommandReceiveBudget,
}
impl Intake {
    pub(super) fn new(budget: CommandReceiveBudget) -> Self {
        Self { budget }
    }
    pub(super) fn budget(&self) -> CommandReceiveBudget {
        self.budget.clone()
    }
    pub(super) fn trusted(
        &self,
        command: ReceivedCommand,
    ) -> Result<ResidentCommand, CoordinatorError> {
        let lane = if command.body().high_priority() {
            IntakeLane::Urgent
        } else {
            IntakeLane::Ordinary
        };
        let lease = self
            .budget
            .try_reserve(lane, command.reservation_bytes(), 1)?;
        Ok(Arc::new(Allocated::new(command, lease)))
    }
    pub(super) fn publication(
        &self,
        command: &ResidentCommand,
    ) -> Result<PublicationWork, CoordinatorError> {
        let lane = if command.value().body().high_priority() {
            IntakeLane::Urgent
        } else {
            IntakeLane::Ordinary
        };
        // Reserve before constructing the owned native Frame/Prompt copy. The
        // bounded metadata reserve plus measured prompt content cover that DTO.
        let copied_frame = self
            .budget
            .try_reserve(lane, command.value().reservation_bytes(), 1)?;
        Ok(PublicationWork {
            budget: self.budget.clone(),
            lane,
            input: command.allocation().clone(),
            copied_frame,
        })
    }
    pub(super) fn acknowledge(
        &self,
        command: &ReceivedCommand,
    ) -> Result<AllocationLease, CoordinatorError> {
        let lane = if command.body().high_priority() {
            IntakeLane::UrgentAcknowledgement
        } else {
            IntakeLane::Acknowledgement
        };
        Ok(self.budget.try_reserve(lane, ACK_BYTES, 0)?)
    }
    pub(super) fn require_resident(command: &ResidentCommand) -> Result<(), CoordinatorError> {
        if command.allocation().bytes() < command.value().reservation_bytes()
            || command.allocation().payloads() == 0
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(())
    }
}
/// Exact reservations for this owner's derived Frame and native JSON output.
/// External public encoding and journal cipher/storage scratch must separately
/// declare/reserve their allocations; these counts do not prove those bounds.
pub(super) struct PublicationWork {
    budget: CommandReceiveBudget,
    lane: IntakeLane,
    input: AllocationLease,
    copied_frame: AllocationLease,
}
impl PublicationWork {
    pub(super) fn encode(
        &self,
        frame: &Frame,
    ) -> Result<(nocterm_cloud_model::payload::OpaqueBytes, AllocationLease), CoordinatorError>
    {
        let encoded = format::encoded_len(frame)?;
        // Vec serialization and Vec -> Arc<[u8]> publication coexist briefly.
        // The counting pass allocates neither an encoded buffer nor serde Value.
        let bytes = encoded.checked_mul(2).ok_or(IntakeError::Capacity)?;
        let output = self.budget.try_reserve(self.lane, bytes, 2)?;
        let allocation = self.combine(output)?;
        Ok((format::encode_counted(frame, encoded)?, allocation))
    }
    fn combine(&self, output: AllocationLease) -> Result<AllocationLease, CoordinatorError> {
        let bytes = self
            .input
            .bytes()
            .checked_add(self.copied_frame.bytes())
            .and_then(|bytes| bytes.checked_add(output.bytes()))
            .ok_or(IntakeError::Capacity)?;
        let payloads = self
            .input
            .payloads()
            .checked_add(self.copied_frame.payloads())
            .and_then(|payloads| payloads.checked_add(output.payloads()))
            .ok_or(IntakeError::Capacity)?;
        Ok(AllocationLease::new(
            bytes,
            payloads,
            (self.input.clone(), self.copied_frame.clone(), output),
        ))
    }
}
/// Allocation is retained through actual remote delivery, but contains no prompt
/// or original error source chain. Dropping this future cancels native delivery;
/// its durable accepted command remains recoverable/retryable by original ID.
pub(super) fn deliver(
    source: Arc<dyn AuthenticatedCommandSource>,
    command: CommandAcknowledgement,
    reply: CommandReply,
    allocation: AllocationLease,
) -> BoxFuture<'static, ()> {
    async move {
        let result = source.acknowledge(command, reply, allocation.clone()).await;
        // Keep the original actual owner through destruction of a returned
        // native error as well as the callee's queued transport operation.
        drop(result);
        drop(allocation);
    }
    .boxed()
}
pub(super) fn reply(result: Result<CommandReceipt, CoordinatorError>) -> CommandReply {
    use CommandRefusal as Refused;
    use CoordinatorError as Error;
    use RetryReason as Retry;
    match result {
        Ok(receipt) => CommandReply::Accepted(receipt),
        Err(error) => match error {
            Error::InvalidCommand => CommandReply::Refused(Refused::InvalidCommand),
            Error::Conflict => CommandReply::Refused(Refused::Conflict),
            Error::Expired => CommandReply::Refused(Refused::Expired),
            Error::Unsupported => CommandReply::Refused(Refused::Unsupported),
            Error::Overloaded | Error::Intake(IntakeError::Capacity) => {
                CommandReply::RetryLater(Retry::Capacity)
            }
            Error::StaleSource => CommandReply::RetryLater(Retry::NotCurrent),
            Error::NeedsGrant => CommandReply::RetryLater(Retry::NeedsGrant),
            Error::NeedsUnlock => CommandReply::RetryLater(Retry::NeedsUnlock),
            Error::NeedsProviderAccess => CommandReply::RetryLater(Retry::NeedsProviderAccess),
            Error::Stopped => CommandReply::RetryLater(Retry::Stopped),
            Error::Journal(JournalError::KeyUnavailable) => {
                CommandReply::RetryLater(Retry::NeedsUnlock)
            }
            _ => CommandReply::RetryLater(Retry::Unavailable),
        },
    }
}
