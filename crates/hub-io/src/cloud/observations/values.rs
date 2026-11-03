//! Inverse descriptive coordinates. No native execution permit is constructed.
use nocterm_cloud_model::{
    authority::{
        AuthenticationBinding, CommandIssuer, ExecutionAuthority, RunBinding, SessionReceipt,
        StreamBinding, TargetBinding,
    },
    coordinator::{ApprovalAnswer, ApprovalChoice, RunStatus},
    engine::UsageObservation,
    identity::*,
    inference::ModelSelection,
    journal::{CommandAcceptance, CommandKind},
    projection::{ProjectionError, RecordKind},
};
use nocterm_hub_model::cloud::{
    self as wire,
    events::{AcceptedCommand, CloudRunStatus, CloudUsage},
    payload as body,
};

pub(super) fn text(value: &wire::CloudTextId) -> Result<NamespaceId, ProjectionError> {
    NamespaceId::new(value.as_str())
        .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Coordinate))
}
pub(super) fn run(
    stream: &StreamBinding,
    value: wire::RunCoordinate,
) -> Result<RunBinding, ProjectionError> {
    Ok(RunBinding::new(
        stream.scope().clone(),
        stream.runner(),
        stream.runner_epoch(),
        stream.chat(),
        RunId::from_bytes(value.id),
        RunEpoch::new(value.expected_epoch)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Coordinate))?,
    ))
}
pub(super) fn model(value: &body::ModelSelection) -> Result<ModelSelection, ProjectionError> {
    Ok(ModelSelection::new(
        text(&value.provider)?,
        text(&value.model)?,
        text(&value.pricing_policy)?,
    ))
}
pub(super) fn target(
    value: &body::TargetAccess,
) -> Result<(TargetBinding, AuthenticationBinding), ProjectionError> {
    Ok((
        TargetBinding::new(
            Fingerprint::from_bytes(value.target.host_key),
            Fingerprint::from_bytes(value.target.route),
            text(&value.target.principal)?,
            Fingerprint::from_bytes(value.target.workspace),
        ),
        AuthenticationBinding::new(
            Fingerprint::from_bytes(value.authentication.public_key),
            Fingerprint::from_bytes(value.authentication.credential_revision),
        ),
    ))
}
pub(super) fn session(value: &body::SessionReference) -> Result<SessionReceipt, ProjectionError> {
    let (target, authentication) = target(&value.access)?;
    Ok(SessionReceipt::new(
        RemoteSessionId::from_bytes(value.id),
        SessionEpoch::new(value.epoch)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Coordinate))?,
        target,
        authentication,
    ))
}
pub(super) fn execution(
    stream: &StreamBinding,
    value: &body::ExecutionReference,
) -> Result<ExecutionAuthority, ProjectionError> {
    Ok(ExecutionAuthority::new(
        run(stream, value.run)?,
        session(&value.session)?,
        GrantId::from_bytes(value.grant.id),
        GrantEpoch::new(value.grant.epoch)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::Coordinate))?,
    ))
}
pub(super) fn accepted(value: &AcceptedCommand) -> Result<CommandAcceptance, ProjectionError> {
    let kind = match value.kind {
        wire::CommandKind::SubmitPrompt => CommandKind::SubmitPrompt,
        wire::CommandKind::Stop => CommandKind::Stop,
        wire::CommandKind::ResolveApproval => CommandKind::ResolveApproval,
        wire::CommandKind::IssueGrant => CommandKind::IssueGrant,
        wire::CommandKind::RevokeGrant => CommandKind::RevokeGrant,
        wire::CommandKind::AnswerSshConsent => CommandKind::AuthenticationConsent,
        wire::CommandKind::DeleteChat => CommandKind::DeleteChat,
        wire::CommandKind::RevokeIssuerGrants => CommandKind::RevokeIssuer,
        // Managed sessions are not accepted by this desktop build yet.
        wire::CommandKind::BeginSession => {
            return Err(ProjectionError::InvalidRecord(RecordKind::ManagedSession));
        }
    };
    Ok(CommandAcceptance::new(
        CommandId::from_bytes(value.id),
        kind,
        Fingerprint::from_bytes(value.signed_message_digest),
        CommandIssuer::new(
            text(&value.issuer.device_id)?,
            Fingerprint::from_bytes(value.issuer.signing_key),
            Fingerprint::from_bytes(value.issuer.access_token_digest),
        ),
        ChatControlRevision::from_u64(value.expected_control_revision),
    ))
}
pub(super) fn approval(
    stream: &StreamBinding,
    value: &body::ApprovalPayload,
) -> Result<ApprovalAnswer, ProjectionError> {
    Ok(ApprovalAnswer::new(
        IntentId::from_bytes(value.intent),
        execution(stream, &value.execution)?,
        Fingerprint::from_bytes(value.arguments_digest),
        match value.choice {
            body::ApprovalChoice::Approve => ApprovalChoice::Approve,
            body::ApprovalChoice::Deny => ApprovalChoice::Deny,
            body::ApprovalChoice::ReconcileSucceeded | body::ApprovalChoice::ReconcileFailed => {
                return Err(ProjectionError::Unsupported);
            }
            body::ApprovalChoice::ReconcileKnown { result_digest } => {
                ApprovalChoice::ReconcileKnown(Fingerprint::from_bytes(result_digest))
            }
        },
    ))
}
pub(super) fn status(value: CloudRunStatus) -> RunStatus {
    match value {
        CloudRunStatus::Queued => RunStatus::Queued,
        CloudRunStatus::Running => RunStatus::Running,
        CloudRunStatus::WaitingApproval => RunStatus::WaitingApproval,
        CloudRunStatus::NeedsGrant => RunStatus::NeedsGrant,
        CloudRunStatus::NeedsUnlock => RunStatus::NeedsUnlock,
        CloudRunStatus::NeedsProviderAccess => RunStatus::NeedsProviderAccess,
        CloudRunStatus::OutcomeUnknown => RunStatus::OutcomeUnknown,
        CloudRunStatus::Interrupted => RunStatus::Interrupted,
        CloudRunStatus::Stopped => RunStatus::Stopped,
        CloudRunStatus::Completed => RunStatus::Completed,
    }
}
pub(super) fn usage(value: CloudUsage) -> UsageObservation {
    UsageObservation {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        total_tokens: value.total_tokens,
        cached_input_tokens: value.cached_input_tokens,
        thought_tokens: value.thought_tokens,
        cost_micros: value.cost_micros,
    }
}
