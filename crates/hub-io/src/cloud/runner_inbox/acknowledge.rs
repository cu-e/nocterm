//! Durable contiguous-prefix publication, separate from informational admission replies.
use super::{InboxOwner, RunnerInboxFault, backend};
use nocterm_cloud_model::coordinator::{
    CommandAcknowledgement, CommandReceiveBudget, CommandReply, CoordinatorError, IntakeError,
    IntakeLane,
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_client::cloud::CloudSelector;
use std::sync::Arc;

const ACK_BYTES: usize = 16 * 1024;
const NATIVE_METADATA_BYTES: usize = 1024;

pub(super) async fn acknowledge(
    owner: Arc<InboxOwner>,
    command: CommandAcknowledgement,
    reply: CommandReply,
    allocation: AllocationLease,
) -> Result<(), CoordinatorError> {
    owner.check_current()?;
    if !owner.material.binding().contains(command.stream()) {
        return Err(CoordinatorError::StaleSource);
    }
    let CommandReply::Accepted(receipt) = reply else {
        // A late retry/refusal cannot revoke another delivery's staged capsule
        // or prove a durable rejection. The prefix hole must remain revisitable.
        return Ok(());
    };
    if receipt.acceptance() != command.acceptance() || receipt.lease().stream() != command.stream()
    {
        return Err(CoordinatorError::InvalidCommand);
    }
    let durable = owner
        .io(async {
            owner
                .journal
                .command_receipt(command.stream().clone(), command.acceptance().id())
                .await
                .map_err(CoordinatorError::from)
        })
        .await?;
    let Some(durable) = durable else {
        return Err(CoordinatorError::InvalidCommand);
    };
    if durable.acceptance() != command.acceptance()
        || durable.lease() != receipt.lease()
        || durable.position() != receipt.position()
    {
        return Err(CoordinatorError::Conflict);
    }
    let removed = {
        let mut state = owner
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let key = (command.stream().chat(), command.acceptance().id());
        match state.pending.get(&key) {
            Some(entry) if entry.command.acceptance() == command.acceptance() => {
                state.pending.remove(&key)
            }
            Some(_) => return Err(CoordinatorError::Conflict),
            None => None,
        }
    };
    let Some(pending) = removed else {
        return Ok(());
    };
    let cursor = pending.cursor;
    drop(pending);
    let _serial = owner.acknowledge.lock().await;
    advance(&owner, cursor)?;
    publish(&owner, allocation).await
}

pub(super) async fn complete_prefix(
    owner: &Arc<InboxOwner>,
    cursor: u64,
    budget: &CommandReceiveBudget,
) -> Result<(), CoordinatorError> {
    let _serial = owner.acknowledge.lock().await;
    if !advance(owner, cursor)? {
        return Ok(());
    }
    publish_reserved(owner, budget).await
}

/// Re-send the same durable prefix after a lost hub ACK or process restart.
/// Never replace it with an untrusted hub floor or a forward scan watermark.
pub(super) async fn sync_confirmed(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
) -> Result<(), CoordinatorError> {
    let Ok(_serial) = owner.acknowledge.try_lock() else {
        return Ok(());
    };
    publish_reserved(owner, budget).await
}

fn advance(owner: &InboxOwner, cursor: u64) -> Result<bool, CoordinatorError> {
    owner.check_current()?;
    let mut state = owner
        .progress
        .lock()
        .map_err(|_| CoordinatorError::NativeTaskFailed)?;
    if cursor > state.cursor.confirmed().saturating_add(1) {
        return Ok(false);
    }
    if let Err(error) = state.cursor.advance(cursor) {
        state.fault = Some(RunnerInboxFault::StorageUnavailable);
        return Err(backend(error));
    }
    Ok(true)
}

async fn publish_reserved(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
) -> Result<(), CoordinatorError> {
    if !needs_publication(owner)? {
        return Ok(());
    }
    match budget.try_reserve(IntakeLane::UrgentAcknowledgement, ACK_BYTES, 0) {
        Ok(allocation) => publish(owner, allocation).await,
        Err(IntakeError::Capacity) => {
            owner.fault(RunnerInboxFault::DeliveryUnconfirmed);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn needs_publication(owner: &InboxOwner) -> Result<bool, CoordinatorError> {
    let state = owner
        .progress
        .lock()
        .map_err(|_| CoordinatorError::NativeTaskFailed)?;
    Ok(state.cursor.confirmed() > state.hub_confirmed)
}

async fn publish(
    owner: &Arc<InboxOwner>,
    allocation: AllocationLease,
) -> Result<(), CoordinatorError> {
    owner.check_current()?;
    let required = nocterm_hub_client::cloud::acknowledgement_work_bytes()
        .checked_add(NATIVE_METADATA_BYTES)
        .ok_or(CoordinatorError::Overloaded)?;
    if allocation.bytes() < required {
        return Err(CoordinatorError::InvalidCommand);
    }
    if !needs_publication(owner)? {
        return Ok(());
    }
    let through = owner
        .progress
        .lock()
        .map_err(|_| CoordinatorError::NativeTaskFailed)?
        .cursor
        .confirmed();
    let allocation = Arc::new(allocation);
    let selector = CloudSelector::RunnerInbox(super::super::metadata_crypto::runner(
        owner.material.binding(),
    ));
    let transport = owner
        .connection
        .capture_transport()
        .await
        .map_err(backend)?;
    let receipt = owner
        .io(async {
            transport
                .during(transport.runner().acknowledge_with_residency(
                    &selector,
                    through,
                    allocation.clone(),
                ))
                .await
                .map_err(backend)?
                .map_err(backend)
        })
        .await;
    // Local ownership covers response validation; actual encoded request and
    // response bodies retain the same real lease independently if cancelled.
    match receipt {
        Ok(confirmed) if confirmed == through => {
            owner.check_current()?;
            let mut state = owner
                .progress
                .lock()
                .map_err(|_| CoordinatorError::NativeTaskFailed)?;
            state.hub_confirmed = state.hub_confirmed.max(confirmed);
            Ok(())
        }
        Ok(_) => {
            owner.fault(RunnerInboxFault::HubGapOrRollback);
            Err(CoordinatorError::InvalidCommand)
        }
        Err(error) => {
            owner.fault(RunnerInboxFault::DeliveryUnconfirmed);
            Err(error)
        }
    }
}
