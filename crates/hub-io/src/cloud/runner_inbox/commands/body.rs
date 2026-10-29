//! Purpose-specific native command bodies, after canonical account/signature validation.
use super::*;
use nocterm_hub_model::cloud::provider::ProviderLeaseContext;

pub(super) fn submit(
    prompt: &wire::PromptPayload<'_>,
    default: &RunDestination,
    issued: i64,
) -> Result<CommandBody, CoordinatorError> {
    let destination = match &prompt.target {
        Some(access) => RunDestination::new(
            target(&access.target)?,
            authentication(access.authentication),
            prompt.existing_session.as_ref().map(session).transpose()?,
        )?,
        None => {
            if prompt.existing_session.is_some() {
                return Err(CoordinatorError::InvalidCommand);
            }
            default.clone()
        }
    };
    let mut content = Vec::with_capacity(prompt.content.len());
    for part in &prompt.content {
        content.push(match part {
            wire::PromptPart::Text(text) => Content::text(*text),
            wire::PromptPart::Image { mime, bytes } => Content::Image(Image::new(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_text(*mime),
            )),
        });
    }
    let allowance = PromptAllowance::new(
        budget(prompt.inference)?,
        prompt.inference.spend_micros,
        GrantWindow::new(timestamp(issued)?, timestamp(prompt.inference.deadline_ms)?)?,
    )?;
    Ok(CommandBody::Submit(PromptRequest::new(
        EnginePrompt::new(content)?,
        model(&prompt.model)?,
        destination,
        allowance,
    )))
}

pub(super) fn approval(
    stream: &StreamBinding,
    answer: wire::ApprovalPayload,
) -> Result<CommandBody, CoordinatorError> {
    let choice = match answer.choice {
        wire::ApprovalChoice::Approve => ApprovalChoice::Approve,
        wire::ApprovalChoice::Deny => ApprovalChoice::Deny,
        wire::ApprovalChoice::ReconcileKnown { result_digest } => {
            ApprovalChoice::ReconcileKnown(Fingerprint::from_bytes(result_digest))
        }
        wire::ApprovalChoice::ReconcileSucceeded | wire::ApprovalChoice::ReconcileFailed => {
            return Err(CoordinatorError::Unsupported);
        }
    };
    Ok(CommandBody::Approval(ApprovalAnswer::new(
        IntentId::from_bytes(answer.intent),
        execution(stream, &answer.execution)?,
        Fingerprint::from_bytes(answer.arguments_digest),
        choice,
    )))
}

pub(super) fn grant(
    grant: wire::GrantPayload<'_>,
    run: &RunBinding,
    origin: GrantOrigin,
) -> Result<CommandBody, CoordinatorError> {
    match grant {
        wire::GrantPayload::Inference {
            grant,
            model: selection,
            window,
            limits,
        } => {
            let task = InferenceTask::new(
                run.clone(),
                model(&selection)?,
                budget(limits)?,
                limits.spend_micros,
                GrantWindow::new(
                    timestamp(window.issued_at_ms)?,
                    timestamp(limits.deadline_ms)?,
                )?,
            )?;
            Ok(CommandBody::InferenceGrant(InferenceGrant::new(
                GrantId::from_bytes(grant.id),
                GrantEpoch::new(grant.epoch)?,
                origin,
                task,
            )))
        }
        wire::GrantPayload::Tools {
            grant,
            session: selected,
            window,
            operations,
            capabilities,
        } => {
            let authority = ExecutionAuthority::new(
                run.clone(),
                session(&selected)?,
                GrantId::from_bytes(grant.id),
                GrantEpoch::new(grant.epoch)?,
            );
            Ok(CommandBody::ToolGrant(GrantPolicy::new(
                authority,
                origin,
                Capabilities::from_bits(capabilities)?,
                GrantWindow::new(
                    timestamp(window.issued_at_ms)?,
                    timestamp(window.expires_at_ms)?,
                )?,
                operations,
            )?))
        }
        wire::GrantPayload::Provider { .. } => Err(CoordinatorError::InvalidCommand),
    }
}

pub(super) fn provider(
    grant: wire::GrantReference,
    context: &ProviderLeaseContext,
    models: &[nocterm_hub_model::cloud::CloudTextId],
) -> Result<ProviderOffer, CoordinatorError> {
    ProviderOffer::new(
        GrantReference::new(
            GrantClass::Provider,
            GrantId::from_bytes(grant.id),
            GrantEpoch::new(grant.epoch)?,
        ),
        namespace(context.provider_reference().as_str())?,
        models
            .iter()
            .map(|value| namespace(value.as_str()))
            .collect::<Result<_, _>>()?,
        Fingerprint::from_bytes(
            Sha256::digest(context.boot().canonical_bytes().map_err(backend)?).into(),
        ),
        Fingerprint::from_bytes(Sha256::digest(context.canonical_bytes().map_err(backend)?).into()),
        timestamp(context.expires_at_ms())?,
    )
}

pub(super) fn ssh(
    answer: &wire::SshConsentAnswer<'_>,
    run: &RunBinding,
) -> Result<SessionConsent, CoordinatorError> {
    let selected = RunDestination::new(
        target(&answer.binding.access.target)?,
        authentication(answer.binding.access.authentication),
        None,
    )?;
    Ok(SessionConsent::new(
        AttemptId::from_bytes(answer.binding.attempt),
        Fingerprint::from_bytes(answer.binding.nonce),
        run.clone(),
        selected,
        namespace("ssh-ed25519")?,
        matches!(answer.decision, wire::SshConsentDecision::Approve { .. }),
        Timestamp::from_millis(answer.binding.expires_ms),
    ))
}
