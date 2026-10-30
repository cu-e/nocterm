//! Sole authenticated public command payload -> native semantic DTO mapping.
mod body;
#[cfg(test)]
mod tests;
use super::attachments::PreparedAttachment;
use base64::Engine as _;
use nocterm_ai::model::content::{Content, Image};
use nocterm_cloud_model::{
    authority::{
        AuthenticationBinding, CommandIssuer, ExecutionAuthority, RunBinding, SessionReceipt,
        StreamBinding, TargetBinding,
    },
    coordinator::*,
    engine::{EnginePrompt, ModelBudget},
    grant::{Capabilities, GrantOrigin, GrantPolicy, GrantWindow},
    identity::*,
    inference::{InferenceGrant, InferenceTask, ModelSelection},
    journal::{CommandAcceptance, CommandKind},
};
use nocterm_core::SharedCause;
use nocterm_hub_model::cloud::{SignedCommand, payload as wire};
use sha2::{Digest, Sha256};

pub(super) struct MappedCommand {
    pub(super) command: ReceivedCommand,
    pub(super) attachment: Option<PreparedAttachment>,
}

pub(super) fn acceptance(
    command: &SignedCommand<'_>,
) -> Result<CommandAcceptance, CoordinatorError> {
    let header = command.header();
    let issuer = header.issuer();
    Ok(CommandAcceptance::new(
        CommandId::from_bytes(header.route().command_id),
        kind(header.context().kind),
        Fingerprint::from_bytes(Sha256::digest(header.canonical_bytes().map_err(backend)?).into()),
        CommandIssuer::new(
            namespace(issuer.device_id.as_str())?,
            Fingerprint::from_bytes(issuer.signing_key),
            Fingerprint::from_bytes(issuer.access_token_digest),
        ),
        ChatControlRevision::from_u64(header.context().expected_chat_revision),
    ))
}
pub(super) fn high_priority(kind: nocterm_hub_model::cloud::CommandKind) -> bool {
    !matches!(kind, nocterm_hub_model::cloud::CommandKind::SubmitPrompt)
}
fn kind(value: nocterm_hub_model::cloud::CommandKind) -> CommandKind {
    use nocterm_hub_model::cloud::CommandKind as Wire;
    match value {
        Wire::SubmitPrompt => CommandKind::SubmitPrompt,
        Wire::Stop => CommandKind::Stop,
        Wire::ResolveApproval => CommandKind::ResolveApproval,
        Wire::IssueGrant => CommandKind::IssueGrant,
        Wire::RevokeGrant => CommandKind::RevokeGrant,
        Wire::AnswerSshConsent => CommandKind::AuthenticationConsent,
        Wire::DeleteChat => CommandKind::DeleteChat,
        Wire::RevokeIssuerGrants => CommandKind::RevokeIssuer,
    }
}

/// Reserve this real generation before native prompt/base64/attachment copies.
/// Prompt JSON size is counted with the existing native AI serializer; no
/// native command is re-encoded to supply its original signed semantic digest.
pub(super) fn native_bytes(
    payload: &wire::CloudCommandPayload<'_>,
    raw_payload: usize,
) -> Result<usize, CoordinatorError> {
    let mut bytes = std::mem::size_of::<ReceivedCommand>()
        .checked_add(64 * 1024)
        .ok_or(CoordinatorError::Overloaded)?;
    if let wire::CloudCommandPayload::SubmitPrompt(prompt) = payload {
        let mut encoded = 1usize;
        for part in &prompt.content {
            let part_bytes = match part {
                wire::PromptPart::Text(text) => {
                    let empty =
                        nocterm_ai::history::content_size(&Content::text("")).map_err(backend)?;
                    empty
                        .checked_sub(2)
                        .and_then(|extra| {
                            extra.checked_add(nocterm_ai::history::text_size(text).ok()?)
                        })
                        .ok_or(CoordinatorError::Overloaded)?
                }
                wire::PromptPart::Image { mime, bytes } => {
                    let encoded = base64::encoded_len(bytes.len(), true)
                        .ok_or(CoordinatorError::Overloaded)?;
                    nocterm_ai::history::content_size(&Content::Image(Image::new(
                        "",
                        mime_text(*mime),
                    )))
                    .map_err(backend)?
                    .checked_add(encoded)
                    .ok_or(CoordinatorError::Overloaded)?
                }
            };
            encoded = encoded
                .checked_add(part_bytes)
                .and_then(|n| n.checked_add(1))
                .ok_or(CoordinatorError::Overloaded)?;
        }
        bytes = bytes
            .checked_add(encoded)
            .and_then(|n| n.checked_add(prompt.content.len() * std::mem::size_of::<Content>()))
            .ok_or(CoordinatorError::Overloaded)?;
    } else if matches!(
        payload,
        wire::CloudCommandPayload::IssueGrant(_) | wire::CloudCommandPayload::AnswerSshConsent(_)
    ) {
        // Original sealed capsule/proof bytes coexist with bounded native metadata.
        bytes = bytes
            .checked_add(raw_payload)
            .ok_or(CoordinatorError::Overloaded)?;
    }
    Ok(bytes)
}

pub(super) fn map(
    signed: &SignedCommand<'_>,
    payload: wire::CloudCommandPayload<'_>,
    stream: StreamBinding,
    acceptance: CommandAcceptance,
    default: &RunDestination,
) -> Result<MappedCommand, CoordinatorError> {
    let context = signed.header().context();
    let window = GrantWindow::new(
        timestamp(context.issued_at_ms)?,
        timestamp(context.accept_before_ms)?,
    )?;
    let run = context.run.map(|value| run(&stream, value)).transpose()?;
    let mut attachment = None;
    let mut hard_expires = None;
    let body = match payload {
        wire::CloudCommandPayload::SubmitPrompt(prompt) => {
            body::submit(&prompt, default, context.issued_at_ms)?
        }
        wire::CloudCommandPayload::Stop => CommandBody::Stop,
        wire::CloudCommandPayload::DeleteChat => CommandBody::DeleteChat,
        wire::CloudCommandPayload::ResolveApproval(answer) => body::approval(&stream, *answer)?,
        wire::CloudCommandPayload::IssueGrant(grant) => {
            let origin = GrantOrigin::new(
                acceptance.issuer().clone(),
                acceptance.id(),
                acceptance.expected_control().next()?,
            );
            match *grant {
                grant @ (wire::GrantPayload::Inference { .. }
                | wire::GrantPayload::Tools { .. }) => body::grant(
                    grant,
                    run.as_ref().ok_or(CoordinatorError::InvalidCommand)?,
                    origin,
                )?,
                wire::GrantPayload::Provider {
                    grant,
                    context,
                    allowed_models,
                    capsule,
                } => {
                    let offer = body::provider(grant, &context, &allowed_models)?;
                    hard_expires = Some(offer.expires());
                    attachment = Some(PreparedAttachment::Provider {
                        context: Box::new(context),
                        capsule: capsule.to_vec(),
                        offer: Box::new(offer.clone()),
                    });
                    CommandBody::ProviderGrant(offer)
                }
            }
        }
        wire::CloudCommandPayload::RevokeGrant(reference) => {
            CommandBody::RevokeGrant(grant_reference(reference)?)
        }
        wire::CloudCommandPayload::RevokeIssuerGrants(revoke) => {
            CommandBody::RevokeIssuer(IssuerRevocation::new(
                IssuerSubject::new(
                    namespace(revoke.device.as_str())?,
                    Fingerprint::from_bytes(revoke.signing_key),
                ),
                ChatControlRevision::from_u64(revoke.control_cutoff),
                revoke
                    .grants
                    .iter()
                    .copied()
                    .map(grant_reference)
                    .collect::<Result<_, _>>()?,
            )?)
        }
        wire::CloudCommandPayload::AnswerSshConsent(answer) => {
            let consent = body::ssh(
                &answer,
                run.as_ref().ok_or(CoordinatorError::InvalidCommand)?,
            )?;
            hard_expires = Some(consent.expires());
            attachment = Some(PreparedAttachment::Ssh {
                payload: signed.payload().to_vec(),
                consent: Box::new(consent.clone()),
            });
            CommandBody::SessionConsent(consent)
        }
    };
    Ok(MappedCommand {
        command: ReceivedCommand::new(stream, acceptance, window, run, body, hard_expires)?,
        attachment,
    })
}
pub(super) fn timestamp(value: i64) -> Result<Timestamp, CoordinatorError> {
    Ok(Timestamp::from_millis(
        u64::try_from(value).map_err(|_| CoordinatorError::InvalidCommand)?,
    ))
}
fn namespace(value: &str) -> Result<NamespaceId, CoordinatorError> {
    NamespaceId::new(value).map_err(backend)
}
fn model(value: &wire::ModelSelection) -> Result<ModelSelection, CoordinatorError> {
    Ok(ModelSelection::new(
        namespace(value.provider.as_str())?,
        namespace(value.model.as_str())?,
        namespace(value.pricing_policy.as_str())?,
    ))
}
fn budget(value: wire::InferenceLimits) -> Result<ModelBudget, CoordinatorError> {
    Ok(ModelBudget::new(
        value.turns,
        usize::try_from(value.request_bytes).map_err(|_| CoordinatorError::InvalidCommand)?,
        usize::try_from(value.output_bytes).map_err(|_| CoordinatorError::InvalidCommand)?,
        value.output_tokens,
    )?
    .with_calls(value.calls)?)
}
pub(super) fn run(
    stream: &StreamBinding,
    value: nocterm_hub_model::cloud::RunCoordinate,
) -> Result<RunBinding, CoordinatorError> {
    Ok(RunBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
        stream.chat(),
        RunId::from_bytes(value.id),
        RunEpoch::new(value.expected_epoch)?,
    ))
}
pub(super) fn target(value: &wire::TargetSelection) -> Result<TargetBinding, CoordinatorError> {
    Ok(TargetBinding::new(
        Fingerprint::from_bytes(value.host_key),
        Fingerprint::from_bytes(value.route),
        namespace(value.principal.as_str())?,
        Fingerprint::from_bytes(value.workspace),
    ))
}
pub(super) fn authentication(value: wire::AuthenticationSelection) -> AuthenticationBinding {
    AuthenticationBinding::new(
        Fingerprint::from_bytes(value.public_key),
        Fingerprint::from_bytes(value.credential_revision),
    )
}
fn session(value: &wire::SessionReference) -> Result<SessionReceipt, CoordinatorError> {
    Ok(SessionReceipt::new(
        RemoteSessionId::from_bytes(value.id),
        SessionEpoch::new(value.epoch)?,
        target(&value.access.target)?,
        authentication(value.access.authentication),
    ))
}
fn execution(
    stream: &StreamBinding,
    value: &wire::ExecutionReference,
) -> Result<ExecutionAuthority, CoordinatorError> {
    Ok(ExecutionAuthority::new(
        run(stream, value.run)?,
        session(&value.session)?,
        GrantId::from_bytes(value.grant.id),
        GrantEpoch::new(value.grant.epoch)?,
    ))
}
fn grant_reference(value: wire::ScopedGrantReference) -> Result<GrantReference, CoordinatorError> {
    let class = match value.class {
        wire::GrantClass::Inference => GrantClass::Inference,
        wire::GrantClass::Tools => GrantClass::Tools,
        wire::GrantClass::Provider => GrantClass::Provider,
    };
    Ok(GrantReference::new(
        class,
        GrantId::from_bytes(value.grant.id),
        GrantEpoch::new(value.grant.epoch)?,
    ))
}
fn mime_text(value: wire::ImageMime) -> &'static str {
    match value {
        wire::ImageMime::Png => "image/png",
        wire::ImageMime::Jpeg => "image/jpeg",
        wire::ImageMime::Gif => "image/gif",
        wire::ImageMime::Webp => "image/webp",
    }
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
