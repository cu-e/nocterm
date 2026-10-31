//! Bounded forward scan plus independent contiguous-prefix probes; holes stay implicit.
use super::{
    InboxOwner, RunnerInboxFault, acknowledge, backend, commands,
    material_store::MaterialStoreError,
};
use nocterm_cloud_model::{
    coordinator::{
        CommandAcknowledgement, CommandInput, CommandReceiveBudget, CoordinatorError, IntakeError,
        IntakeLane, RetryReason,
    },
    identity::{ChatKeyEpoch, CloudChatId},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_hub_client::cloud::{
    CloudDeliveryOrigin, CloudError, CloudReplay, CloudReplayItem, CloudSelector,
};
use nocterm_hub_model::cloud::{
    payload::CloudCommandPayload,
    relay::{PublicationMetadata, RelayKind},
};
use nocterm_keys::{PublicKey, cloud::open_command};
use std::sync::Arc;

enum Scanned {
    Input(CommandInput),
    Handled,
    Empty,
}
struct Incoming {
    metadata: PublicationMetadata,
    origin: CloudDeliveryOrigin,
    cursor: u64,
    ciphertext: Vec<u8>,
    _allocation: Arc<AllocationLease>,
}

pub(super) async fn receive(
    owner: Arc<InboxOwner>,
    budget: CommandReceiveBudget,
) -> Result<Option<CommandInput>, CoordinatorError> {
    let _turn = owner.receive.lock().await;
    owner.check_current()?;
    {
        let mut state = owner
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        if state
            .budget
            .as_ref()
            .is_some_and(|previous| !previous.same_owner(&budget))
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        state.budget = Some(budget.clone());
    }
    loop {
        owner.check_current()?;
        owner
            .connection
            .wait_transport_ready()
            .await
            .map_err(backend)?;
        owner.prune_pending()?;
        if acknowledge::sync_confirmed(&owner, &budget).await.is_err() {
            // ACK loss must not prevent a separately available urgent replay path.
            // The durable prefix stays pending and is resent verbatim later.
            owner.check_current()?;
            owner.fault(RunnerInboxFault::DeliveryUnconfirmed);
        }
        if let Some(input) = scan_window(&owner, &budget).await? {
            return Ok(Some(input));
        }
        // No assembled/opened payload is retained during this genuine idle wait.
        // Empty transport replay is not the authenticated source's end-of-stream.
        owner.check_current()?;
        owner.timer.delay(owner.options.interval).await;
        owner.check_current()?;
    }
}

/// At most eight records per wake. None is an idle WINDOW, never a source close.
/// Forward/probe positions survive waits, including unacknowledged capacity holes.
async fn scan_window(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
) -> Result<Option<CommandInput>, CoordinatorError> {
    for _ in 0..8 {
        owner.check_current()?;
        let (after, probe) = {
            let mut state = owner
                .progress
                .lock()
                .map_err(|_| CoordinatorError::NativeTaskFailed)?;
            if state.probes >= owner.options.probe_every {
                state.probes = 0;
                (state.cursor.confirmed(), true)
            } else {
                state.probes += 1;
                (state.forward, false)
            }
        };
        match scan(owner, budget, after, probe).await {
            Ok(Scanned::Input(input)) => return Ok(Some(input)),
            Ok(Scanned::Handled) => {}
            Ok(Scanned::Empty) => {
                if !probe {
                    let mut state = owner
                        .progress
                        .lock()
                        .map_err(|_| CoordinatorError::NativeTaskFailed)?;
                    state.forward = state.cursor.confirmed();
                }
                break;
            }
            Err(error) => {
                if owner
                    .progress
                    .lock()
                    .map_err(|_| CoordinatorError::NativeTaskFailed)?
                    .fault
                    .is_some_and(RunnerInboxFault::fatal)
                {
                    return Err(error);
                }
                if matches!(
                    error,
                    CoordinatorError::StaleSource | CoordinatorError::Expired
                ) {
                    return Err(error);
                }
                owner.fault(RunnerInboxFault::DeliveryUnconfirmed);
                break;
            }
        }
    }
    Ok(None)
}

async fn scan(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
    after: u64,
    probe: bool,
) -> Result<Scanned, CoordinatorError> {
    let incoming = read(owner, budget, after).await?;
    let Some(incoming) = incoming else {
        return Ok(Scanned::Empty);
    };
    owner.check_current()?;
    if !probe {
        let mut state = owner
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        state.forward = state.forward.max(incoming.cursor);
    }
    match incoming.metadata.binding().kind() {
        RelayKind::ChatKeyGrant => material(owner, budget, incoming).await,
        RelayKind::Command => command(owner, budget, incoming).await,
        _ => {
            owner.fault(RunnerInboxFault::Authentication);
            Err(CoordinatorError::InvalidCommand)
        }
    }
}

async fn read(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
    after: u64,
) -> Result<Option<Incoming>, CoordinatorError> {
    let transport = owner
        .connection
        .capture_transport()
        .await
        .map_err(backend)?;
    // One native ciphertext generation and one opened plaintext generation.
    // Separate fixed slack covers bounded semantic parser arrays/header hashing;
    // final native DTO/attachment copies reserve their own ordinary/urgent lane.
    let bytes = nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES
        .checked_mul(2)
        .and_then(|n| n.checked_add(CloudReplay::buffer_work_bytes()))
        .and_then(|n| n.checked_add(128 * 1024))
        .ok_or(CoordinatorError::Overloaded)?;
    let allocation = Arc::new(budget.try_reserve(IntakeLane::Ingress, bytes, 2)?);
    let selector = CloudSelector::RunnerInbox(super::super::metadata_crypto::runner(
        owner.material.binding(),
    ));
    let mut replay = rpc(
        owner,
        &transport,
        transport.runner().replay_with_residency(
            selector,
            after,
            1,
            u32::try_from(nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES)
                .map_err(|_| CoordinatorError::Overloaded)?,
            allocation.clone(),
        ),
    )
    .await?;
    let mut record = match rpc(owner, &transport, replay.next_record()).await? {
        CloudReplayItem::Record(record) => record,
        CloudReplayItem::End { .. } => return Ok(None),
        CloudReplayItem::Gap { .. } => {
            owner.fault(RunnerInboxFault::HubGapOrRollback);
            return Err(CoordinatorError::InvalidCommand);
        }
    };
    let metadata = record.metadata().clone();
    let expected =
        super::super::metadata_crypto::namespace(owner.material.binding()).map_err(backend)?;
    if metadata.binding().namespace() != &expected
        || metadata.binding().runner()
            != super::super::metadata_crypto::runner(owner.material.binding())
    {
        owner.fault(RunnerInboxFault::Authentication);
        return Err(CoordinatorError::InvalidCommand);
    }
    let cursor = record.cursor();
    let origin = record.origin().clone();
    let length =
        usize::try_from(metadata.total_bytes()).map_err(|_| CoordinatorError::Overloaded)?;
    if length > nocterm_hub_model::cloud::MAX_ENVELOPE_BYTES {
        return Err(CoordinatorError::Overloaded);
    }
    let ciphertext = ciphertext(owner, &transport, &mut record, length).await?;
    drop(record);
    if ciphertext.len() != length {
        owner.fault(RunnerInboxFault::Authentication);
        return Err(CoordinatorError::InvalidCommand);
    }
    if !matches!(rpc(owner, &transport, replay.next_record()).await?,
        CloudReplayItem::End { through, .. } if through == cursor)
    {
        owner.fault(RunnerInboxFault::Authentication);
        return Err(CoordinatorError::InvalidCommand);
    }
    Ok(Some(Incoming {
        metadata,
        origin,
        cursor,
        ciphertext,
        _allocation: allocation,
    }))
}

async fn ciphertext(
    owner: &InboxOwner,
    transport: &super::super::runner_peer::CapturedTransport,
    record: &mut nocterm_hub_client::cloud::CloudIncomingRecord<'_>,
    length: usize,
) -> Result<Vec<u8>, CoordinatorError> {
    let mut ciphertext = Vec::with_capacity(length);
    loop {
        let chunk = rpc(owner, transport, record.next_chunk()).await?;
        let Some(chunk) = chunk else {
            break;
        };
        if ciphertext
            .len()
            .checked_add(chunk.len())
            .is_none_or(|next| next > length)
        {
            owner.fault(RunnerInboxFault::Authentication);
            return Err(CoordinatorError::InvalidCommand);
        }
        ciphertext.extend_from_slice(&chunk);
    }
    Ok(ciphertext)
}

async fn rpc<T>(
    owner: &InboxOwner,
    transport: &super::super::runner_peer::CapturedTransport,
    future: impl std::future::Future<Output = Result<T, CloudError>>,
) -> Result<T, CoordinatorError> {
    owner
        .io(async {
            let response = transport.during(future).await.map_err(backend)?;
            response.map_err(|error| wire_error(owner, transport, error))
        })
        .await
}

async fn material(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
    incoming: Incoming,
) -> Result<Scanned, CoordinatorError> {
    if !matches!(&incoming.origin, CloudDeliveryOrigin::Device { account, .. }
        if account.as_str() == owner.material.binding().scope().account().id().as_str())
    {
        owner.fault(RunnerInboxFault::Authentication);
        return Err(CoordinatorError::InvalidCommand);
    }
    let coordinate = incoming
        .metadata
        .binding()
        .material_coordinate()
        .ok_or(CoordinatorError::InvalidCommand)?;
    let stream = owner
        .material
        .binding()
        .stream(CloudChatId::from_bytes(coordinate.chat_id()));
    let epoch = ChatKeyEpoch::new(coordinate.key_epoch())?;
    owner.check_current()?;
    let admitted = owner.material.admit(
        &stream,
        epoch,
        owner.principal.account,
        incoming.metadata.id(),
        &incoming.ciphertext,
    );
    owner.check_current()?;
    let opened = chat_opened(&admitted, &stream);
    match admitted {
        Ok(()) | Err(MaterialStoreError::Retired) => {
            // A tombstoned generation has an explicit durable no-revival policy.
            // admit authenticated the actual account box before selecting it.
            acknowledge::complete_prefix(owner, incoming.cursor, budget).await?;
        }
        Err(
            MaterialStoreError::Conflict
            | MaterialStoreError::Capacity
            | MaterialStoreError::Unavailable
            | MaterialStoreError::Storage(_)
            | MaterialStoreError::StoredButUnavailable(_),
        ) => {
            owner.fault(RunnerInboxFault::StorageUnavailable);
        }
        Err(error) => {
            owner.fault(RunnerInboxFault::Authentication);
            return Err(backend(error));
        }
    }
    Ok(opened.map_or(Scanned::Handled, Scanned::Input))
}

/// Only an installed (or already installed) key opens the chat. A retired
/// generation has a durable no-revival policy and opens nothing.
fn chat_opened(
    admitted: &Result<(), MaterialStoreError>,
    stream: &nocterm_cloud_model::authority::StreamBinding,
) -> Option<CommandInput> {
    admitted.is_ok().then(|| CommandInput::ChatOpened {
        stream: stream.clone(),
    })
}

/// The delivery origin must be the issuing device of the account's runner.
fn check_origin(
    owner: &InboxOwner,
    origin: &CloudDeliveryOrigin,
    signed: &nocterm_hub_model::cloud::SignedCommand<'_>,
) -> Result<(), CoordinatorError> {
    match origin {
        CloudDeliveryOrigin::Device {
            account,
            device,
            key,
        } if account.as_str() == owner.material.binding().scope().account().id().as_str()
            && device == &signed.header().issuer().device_id
            && key == &signed.header().issuer().signing_key =>
        {
            Ok(())
        }
        _ => {
            owner.fault(RunnerInboxFault::Authentication);
            Err(CoordinatorError::InvalidCommand)
        }
    }
}
async fn command(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
    incoming: Incoming,
) -> Result<Scanned, CoordinatorError> {
    let route = incoming
        .metadata
        .binding()
        .command_route()
        .ok_or(CoordinatorError::InvalidCommand)?;
    let account = PublicKey::from_bytes(&owner.principal.account).map_err(backend)?;
    let opened = owner
        .material
        .identity()
        .with_keys(|keys| open_command(keys.encryption(), &account, route, &incoming.ciphertext))
        .map_err(backend)?
        .map_err(|error| {
            owner.fault(RunnerInboxFault::Authentication);
            backend(error)
        })?;
    let signed = opened.command().map_err(backend)?;
    check_origin(owner, &incoming.origin, &signed)?;
    let stream = owner
        .material
        .binding()
        .stream(CloudChatId::from_bytes(route.chat_id));
    let acceptance = commands::acceptance(&signed)?;
    let acknowledgement = CommandAcknowledgement::new(stream.clone(), acceptance.clone());
    let durable = owner
        .io(async {
            owner
                .journal
                .command_receipt(stream.clone(), acceptance.id())
                .await
                .map_err(CoordinatorError::from)
        })
        .await;
    match durable {
        Ok(Some(receipt))
            if receipt.acceptance() == &acceptance && receipt.lease().stream() == &stream =>
        {
            acknowledge::complete_prefix(owner, incoming.cursor, budget).await?;
            return Ok(Scanned::Handled);
        }
        Ok(Some(_)) => {
            owner.fault(RunnerInboxFault::InvalidCommand);
            return Ok(Scanned::Handled);
        }
        Err(_) => {
            return deferred(
                owner,
                budget,
                acknowledgement,
                signed.header().context().kind,
                RetryReason::Unavailable,
            );
        }
        Ok(None) => {}
    }
    if owner.inflight(&acknowledgement)? {
        return Ok(Scanned::Handled);
    }
    let now = i64::try_from(owner.attachments.check_current()?.as_millis())
        .map_err(|_| CoordinatorError::Expired)?;
    if signed.header().validate_admission_time(now).is_err() {
        owner.fault(RunnerInboxFault::InvalidCommand);
        return Ok(Scanned::Handled);
    }
    let Ok(payload) = CloudCommandPayload::from_command(&signed) else {
        owner.fault(RunnerInboxFault::InvalidCommand);
        return Ok(Scanned::Handled);
    };
    prepare(
        owner,
        budget,
        incoming.cursor,
        &signed,
        payload,
        stream,
        acceptance,
    )
}

fn prepare(
    owner: &Arc<InboxOwner>,
    budget: &CommandReceiveBudget,
    cursor: u64,
    signed: &nocterm_hub_model::cloud::SignedCommand<'_>,
    payload: CloudCommandPayload<'_>,
    stream: nocterm_cloud_model::authority::StreamBinding,
    acceptance: nocterm_cloud_model::journal::CommandAcceptance,
) -> Result<Scanned, CoordinatorError> {
    let acknowledgement = CommandAcknowledgement::new(stream.clone(), acceptance.clone());
    let high = commands::high_priority(signed.header().context().kind);
    let lane = if high {
        IntakeLane::Urgent
    } else {
        IntakeLane::Ordinary
    };
    let bytes = commands::native_bytes(&payload, signed.payload().len())?;
    let allocation = match budget.try_reserve(lane, bytes, 1) {
        Ok(allocation) => allocation,
        Err(IntakeError::Capacity) => {
            return deferred(
                owner,
                budget,
                acknowledgement,
                signed.header().context().kind,
                RetryReason::Capacity,
            );
        }
        Err(error) => return Err(error.into()),
    };
    let attachment_allocation = if needs_attachment(&payload) {
        match owner.attachments.reserve(signed.payload().len()) {
            Ok(allocation) => Some(allocation),
            Err(CoordinatorError::Overloaded) => {
                return deferred(
                    owner,
                    budget,
                    acknowledgement,
                    signed.header().context().kind,
                    RetryReason::Capacity,
                );
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let mapped = match commands::map(
        signed,
        payload,
        stream,
        acceptance,
        &owner.principal.default,
    ) {
        Ok(value) => value,
        Err(_) => {
            owner.fault(RunnerInboxFault::InvalidCommand);
            return Ok(Scanned::Handled);
        }
    };
    if allocation.bytes() < mapped.command.reservation_bytes() {
        return Err(CoordinatorError::InvalidCommand);
    }
    let ticket = if let Some(body) = mapped.attachment {
        let staged = owner.attachments.stage(
            mapped.command.acknowledgement(),
            body,
            attachment_allocation.ok_or(CoordinatorError::InvalidCommand)?,
        );
        match staged {
            Ok(ticket) => Some(ticket),
            Err(_) => {
                owner.fault(RunnerInboxFault::InvalidCommand);
                return Ok(Scanned::Handled);
            }
        }
    } else {
        None
    };
    let resident = Arc::new(Allocated::new(mapped.command, allocation));
    if let Err(error) = owner.remember(&resident, cursor) {
        if let Some(ticket) = ticket {
            owner.attachments.discard_ticket(ticket);
        }
        return Err(error);
    }
    owner.check_current()?;
    Ok(Scanned::Input(CommandInput::Ready(resident)))
}
/// Provider grants and SSH consent answers stage an attachment capsule.
fn needs_attachment(payload: &CloudCommandPayload<'_>) -> bool {
    match payload {
        CloudCommandPayload::IssueGrant(grant) => matches!(
            grant.as_ref(),
            nocterm_hub_model::cloud::payload::GrantPayload::Provider { .. }
        ),
        CloudCommandPayload::AnswerSshConsent(_) => true,
        _ => false,
    }
}
fn deferred(
    owner: &InboxOwner,
    budget: &CommandReceiveBudget,
    command: CommandAcknowledgement,
    kind: nocterm_hub_model::cloud::CommandKind,
    reason: RetryReason,
) -> Result<Scanned, CoordinatorError> {
    let lane = if commands::high_priority(kind) {
        IntakeLane::UrgentAcknowledgement
    } else {
        IntakeLane::Acknowledgement
    };
    match budget.try_reserve(lane, 16 * 1024, 0) {
        Ok(allocation) => Ok(Scanned::Input(CommandInput::Deferred {
            command: Box::new(command),
            reason,
            allocation,
        })),
        Err(IntakeError::Capacity) => {
            owner.fault(RunnerInboxFault::DeliveryUnconfirmed);
            Ok(Scanned::Handled)
        }
        Err(error) => Err(error.into()),
    }
}
fn wire_error(
    owner: &InboxOwner,
    transport: &super::super::runner_peer::CapturedTransport,
    error: CloudError,
) -> CoordinatorError {
    // An old token generation cannot poison the original runtime fault state.
    if let Err(changed) = transport.check() {
        return backend(changed);
    }
    let fault = match &error {
        CloudError::Protocol
        | CloudError::Relay(_)
        | CloudError::Command(_)
        | CloudError::Encryption(_) => RunnerInboxFault::Authentication,
        CloudError::SourceChanged | CloudError::NotSignedIn => RunnerInboxFault::NeedsRunnerAccess,
        _ => RunnerInboxFault::DeliveryUnconfirmed,
    };
    owner.fault(fault);
    backend(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::{authority::StreamBinding, identity::*};

    fn stream() -> StreamBinding {
        StreamBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new("hub").unwrap(),
                    Fingerprint::from_bytes([1; 32]),
                ),
                AccountIdentity::new(
                    NamespaceId::new("account").unwrap(),
                    Fingerprint::from_bytes([2; 32]),
                ),
            ),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
            CloudChatId::from_bytes([5; 16]),
        )
    }

    #[test]
    fn an_admitted_chat_key_opens_its_chat() {
        let opened = chat_opened(&Ok(()), &stream());
        assert!(matches!(
            opened,
            Some(CommandInput::ChatOpened { stream: opened }) if opened == stream()
        ));
    }

    #[test]
    fn a_retired_or_refused_chat_key_opens_nothing() {
        for refusal in [
            MaterialStoreError::Retired,
            MaterialStoreError::Conflict,
            MaterialStoreError::Capacity,
        ] {
            assert!(chat_opened(&Err(refusal), &stream()).is_none());
        }
    }
}
