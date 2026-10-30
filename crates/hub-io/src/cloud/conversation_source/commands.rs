//! Stage once before transport; every retry binds the original authenticated M.
use super::{
    SourceCore, heads,
    outbox::{StoredCommand, intent_fingerprint},
    requirements, sdk_error,
};
use futures::future::BoxFuture;
use nocterm_cloud_model::projection::ConversationIntent;
use nocterm_cloud_model::{
    identity::*,
    projection::{ProjectionError, ports::*},
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_hub_model::cloud::{
    CloudRoute, CommandContext, CommandKind, RunCoordinate, RunnerCoordinate,
    head::{HeadTarget, HeadValue, ReceiptOutcome},
    payload::CloudCommandPayload,
};
use nocterm_keys::{PublicKey, random_bytes};
use std::sync::Arc;

pub(super) fn prepare(
    core: &SourceCore,
    coordinates: ConversationCoordinates,
    action: ConversationIntent,
) -> Result<ConversationCommand, ProjectionError> {
    core.check()?;
    if coordinates.binding != core.binding
        || coordinates
            .run
            .as_ref()
            .is_some_and(|run| run.stream() != *core.binding.stream())
    {
        return Err(ProjectionError::WrongSource);
    }
    if matches!(&action, ConversationIntent::Stop) && coordinates.run.is_none() {
        return Err(ProjectionError::WrongRun);
    }
    if matches!(&action, ConversationIntent::Prompt(_)) && !core.policy.allows_prompt() {
        return Err(ProjectionError::Unavailable);
    }
    if let ConversationIntent::AnswerApproval(request) = &action {
        if coordinates.run.as_ref() != Some(request.answer().authority().run()) {
            return Err(ProjectionError::WrongRun);
        }
    }
    let invocation = core
        .native
        .cloud()
        .command_invocation()
        .map_err(sdk_error)?;
    let issued = core.now()?;
    let preparation = core.policy.preparation(
        issued,
        invocation.subject().hard_refresh_expires_at_ms(),
        &action,
    )?;
    Ok(ConversationCommand {
        id: CommandId::from_bytes(random_bytes().map_err(|_| ProjectionError::Unavailable)?),
        coordinates,
        action,
        preparation,
    })
}
pub(super) fn stage(
    core: Arc<SourceCore>,
    command: Arc<Allocated<ConversationCommand>>,
    work: AllocationLease,
) -> BoxFuture<'static, Result<(), ProjectionError>> {
    Box::pin(async move {
        core.require_work(&work, requirements::stage(command.value())?)?;
        {
            let mut progress = core
                .progress
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?;
            let intent = intent_fingerprint(command.value());
            match progress.attempts.get(&command.value().id) {
                Some(old) if old.intent != intent => return Err(ProjectionError::InvalidRecord),
                Some(_) => {}
                None => {
                    if progress.attempts.len()
                        >= if matches!(&command.value().action, ConversationIntent::Stop) {
                            super::storage::MAX_PENDING
                        } else {
                            super::storage::MAX_PENDING - 1
                        }
                    {
                        return Err(ProjectionError::Capacity);
                    }
                    progress.attempts.insert(
                        command.value().id,
                        super::StageAttempt {
                            intent,
                            unsent: true,
                            urgent: matches!(&command.value().action, ConversationIntent::Stop),
                        },
                    );
                }
            }
        }
        let native = core.native.clone();
        let workers = core.workers.clone();
        let output = workers
            .blocking(
                work.clone(),
                matches!(&command.value().action, ConversationIntent::Stop),
                move || stage_native(&core, command.value(), work.bytes()),
            )?
            .await
            .map_err(|_| ProjectionError::Unavailable)?;
        native.check()?;
        output.result().copied()
    })
}
fn stage_native(
    core: &SourceCore,
    command: &ConversationCommand,
    work_bytes: usize,
) -> Result<(), ProjectionError> {
    core.check()?;
    core.now()?;
    if command.coordinates.binding != core.binding {
        return Err(ProjectionError::WrongSource);
    }
    let outbox = core
        .files
        .outbox()
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?;
    if let Some(cell) = outbox.read(command.id, requirements::read_capacity(work_bytes)?)? {
        let chat = core.chat()?;
        let stored = core
            .native
            .material()
            .with_key(|key| {
                Ok(StoredCommand::open(
                    key,
                    chat,
                    command.id,
                    core.binding.writer(),
                    &cell,
                ))
            })
            .map_err(|_| ProjectionError::Stale)??;
        return if stored.intent == intent_fingerprint(command) {
            Ok(())
        } else {
            Err(ProjectionError::InvalidRecord)
        };
    }
    if core
        .progress
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?
        .attempts
        .get(&command.id)
        .is_some_and(|attempt| !attempt.unsent)
    {
        return Err(ProjectionError::OutcomeUnknown);
    }
    if command.preparation.policy() != core.policy.fingerprint() {
        return Err(ProjectionError::WrongSource);
    }
    let sealed = seal_stored(core, command)?;
    let publication = outbox.stage(
        command.id,
        &sealed,
        core.binding.writer().get(),
        matches!(&command.action, ConversationIntent::Stop),
    )?;
    let file = core.publish(|| {
        publication
            .publish_new()
            .map_err(|_| ProjectionError::Persistence)
    })??;
    outbox.confirm(&file)?;
    if let Some(attempt) = core
        .progress
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?
        .attempts
        .get_mut(&command.id)
    {
        attempt.unsent = false;
    }
    core.check()
}
fn seal(
    core: &SourceCore,
    command: &ConversationCommand,
) -> Result<nocterm_hub_client::cloud::PreparedCloudCommand, ProjectionError> {
    if core.now()? >= command.preparation.expires() {
        return Err(ProjectionError::Unavailable);
    }
    let stream = core.binding.stream();
    let route = CloudRoute {
        namespace: core.native.cloud().namespace().clone(),
        runner: RunnerCoordinate {
            id: *stream.runner().as_bytes(),
            key_epoch: stream.runner_epoch().get(),
        },
        chat_id: *stream.chat().as_bytes(),
        command_id: *command.id.as_bytes(),
    };
    let kind = match &command.action {
        ConversationIntent::Prompt(_) => CommandKind::SubmitPrompt,
        ConversationIntent::Stop => CommandKind::Stop,
        ConversationIntent::AnswerApproval(_) => CommandKind::ResolveApproval,
    };
    let context = CommandContext {
        nonce: random_bytes().map_err(|_| ProjectionError::Unavailable)?,
        expected_chat_revision: command.coordinates.control.get(),
        run: match kind {
            CommandKind::SubmitPrompt => None,
            _ => command.coordinates.run.as_ref().map(|run| RunCoordinate {
                id: *run.run().as_bytes(),
                expected_epoch: run.epoch().get(),
            }),
        },
        kind,
        issued_at_ms: i64::try_from(command.preparation.issued().as_millis())
            .map_err(|_| ProjectionError::InvalidRecord)?,
        accept_before_ms: i64::try_from(command.preparation.expires().as_millis())
            .map_err(|_| ProjectionError::InvalidRecord)?,
    };
    let payload = payload(&core.policy, &command.action, command.preparation)?;
    let runner = PublicKey::from_bytes(&core.native.material().request().pins.encryption)
        .map_err(|_| ProjectionError::WrongSource)?;
    core.native.with_keyset(|keys| {
        core.native
            .cloud()
            .prepare_payload(keys, &runner, route, context, &payload)
    })
}
pub(super) fn deliver(
    core: Arc<SourceCore>,
    id: CommandId,
    work: AllocationLease,
) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
    Box::pin(async move {
        let stored = load(core.clone(), id, work.clone()).await?;
        let value = stored.value();
        let prepared = core
            .native
            .cloud()
            .stage_stored_command(
                value.metadata.clone(),
                value.ciphertext.clone(),
                value.semantic,
            )
            .map_err(sdk_error)?;
        let guard = Arc::new((stored, work.clone()));
        let mut upload = core
            .native
            .cloud()
            .begin_publication_with_residency(prepared.metadata().clone(), guard)
            .map_err(sdk_error)?;
        let frame = usize::try_from(core.native.cloud().limits().max_frame_bytes)
            .map_err(|_| ProjectionError::Capacity)?;
        if frame == 0 {
            return Err(ProjectionError::InvalidRecord);
        }
        for bytes in prepared.ciphertext().chunks(frame) {
            core.check()?;
            upload.write(bytes).await.map_err(sdk_error)?;
            core.check()?;
        }
        let receipt = upload.finish().await.map_err(sdk_error)?;
        core.check()?;
        if receipt.publication_id.as_slice() != prepared.metadata().id()
            || receipt.ciphertext_sha256.as_slice() != prepared.metadata().digest()
            || receipt.cursor == 0
        {
            return Err(ProjectionError::InvalidRecord);
        }
        // The hub durably stored bytes. This says NOTHING about R admission.
        Ok(CommandDelivery::Delivered)
    })
}
pub(super) fn query(
    core: Arc<SourceCore>,
    id: CommandId,
    work: AllocationLease,
) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
    Box::pin(async move {
        let stored = match load(core.clone(), id, work.clone()).await {
            Ok(stored) => stored,
            Err(ProjectionError::OutcomeUnknown)
                if core
                    .progress
                    .lock()
                    .map_err(|_| ProjectionError::Unavailable)?
                    .attempts
                    .get(&id)
                    .is_some_and(|attempt| attempt.unsent) =>
            {
                return Ok(CommandDelivery::NotFound);
            }
            Err(error) => return Err(error),
        };
        let value = stored.value();
        let request = core.native.material().request();
        let witness = heads::probe(
            &core.native,
            &core.clock,
            HeadTarget::CommandReceipt {
                chat_id: *request.stream.chat().as_bytes(),
                chat_key_epoch: request.epoch.get(),
                command_id: *id.as_bytes(),
                semantic_digest: value.semantic,
                expected_control_revision: value.control,
            },
            &heads::ProbeWork::new(work)?,
        )
        .await?;
        core.check()?;
        let HeadValue::CommandReceipt(outcome) = &witness.witness().witness().value else {
            return Err(ProjectionError::InvalidRecord);
        };
        Ok(match outcome {
            ReceiptOutcome::NotFound => CommandDelivery::NotFound,
            ReceiptOutcome::Accepted { .. } => CommandDelivery::Accepted,
            ReceiptOutcome::Conflict => CommandDelivery::Rejected,
            ReceiptOutcome::Unknown => CommandDelivery::Pending,
        })
    })
}
async fn load(
    core: Arc<SourceCore>,
    id: CommandId,
    work: AllocationLease,
) -> Result<Allocated<StoredCommand>, ProjectionError> {
    core.require_work(&work, heads::work_bytes())?;
    let maximum = requirements::read_capacity(work.bytes())?;
    let workers = core.workers.clone();
    let native = core.native.clone();
    let urgent = core
        .progress
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?
        .attempts
        .get(&id)
        .is_some_and(|attempt| attempt.urgent);
    let output = workers
        .blocking(work, urgent, move || {
            let outbox = core
                .files
                .outbox()
                .lock()
                .map_err(|_| ProjectionError::Unavailable)?;
            let cell = outbox
                .read(id, maximum)?
                .ok_or(ProjectionError::OutcomeUnknown)?;
            let chat = core.chat()?;
            let stored = core
                .native
                .material()
                .with_key(|key| {
                    Ok(StoredCommand::open(
                        key,
                        chat,
                        id,
                        core.binding.writer(),
                        &cell,
                    ))
                })
                .map_err(|_| ProjectionError::Stale)??;
            let route = stored
                .metadata
                .binding()
                .command_route()
                .ok_or(ProjectionError::InvalidRecord)?;
            let stream = core.binding.stream();
            if route.namespace != *core.native.cloud().namespace()
                || route.runner.id != *stream.runner().as_bytes()
                || route.runner.key_epoch != stream.runner_epoch().get()
                || route.chat_id != *stream.chat().as_bytes()
            {
                return Err(ProjectionError::WrongSource);
            }
            core.check()?;
            Ok(stored)
        })?
        .await
        .map_err(|_| ProjectionError::Unavailable)?;
    native.check()?;
    output.into_allocated()
}

pub(super) fn retire(core: &SourceCore, id: CommandId) -> Result<(), ProjectionError> {
    core.check()?;
    let outbox = core
        .files
        .outbox()
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?;
    core.publish(|| outbox.archive(id))??;
    core.progress
        .lock()
        .map_err(|_| ProjectionError::Unavailable)?
        .attempts
        .remove(&id);
    Ok(())
}

fn seal_stored(
    core: &SourceCore,
    command: &ConversationCommand,
) -> Result<Vec<u8>, ProjectionError> {
    let prepared = seal(core, command)?;
    let stored = StoredCommand {
        intent: intent_fingerprint(command),
        semantic: prepared.semantic_digest(),
        control: command.coordinates.control.get(),
        metadata: prepared.metadata().clone(),
        ciphertext: prepared.ciphertext().to_vec(),
    };
    let chat = core.chat()?;
    core.native
        .material()
        .with_key(|key| Ok(stored.seal(key, chat, command.id, core.binding.writer())))
        .map_err(|_| ProjectionError::Stale)?
}

pub(super) fn payload<'a>(
    policy: &super::policy::ConversationPolicy,
    action: &'a ConversationIntent,
    preparation: ConversationPreparation,
) -> Result<CloudCommandPayload<'a>, ProjectionError> {
    Ok(match action {
        ConversationIntent::Prompt(text) => policy.prompt(text, preparation)?,
        ConversationIntent::Stop => CloudCommandPayload::Stop,
        ConversationIntent::AnswerApproval(request) => {
            CloudCommandPayload::ResolveApproval(Box::new(
                crate::cloud::events::values::approval(request.answer())
                    .map_err(|_| ProjectionError::InvalidRecord)?,
            ))
        }
    })
}
