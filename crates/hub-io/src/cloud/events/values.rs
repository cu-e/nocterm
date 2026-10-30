//! SDK-only descriptive mappings. These values never restore execution authority.
use super::encoding_error;
use nocterm_cloud_model::{
    authority::{
        AuthenticationBinding, ExecutionAuthority, RunBinding, SessionReceipt, TargetBinding,
    },
    coordinator::{ApprovalAnswer, ApprovalChoice, RunStatus},
    engine::UsageObservation,
    identity::NamespaceId,
    inference::ModelSelection,
    journal::{CommandAcceptance, CommandKind, JournalError},
};
use nocterm_hub_model::cloud::{
    self as wire,
    events::{AcceptedCommand, CloudRunStatus, CloudUsage},
    payload as body,
};

pub(super) fn text(value: &NamespaceId) -> Result<wire::CloudTextId, JournalError> {
    wire::CloudTextId::new(value.as_str()).map_err(encoding_error)
}
pub(super) fn run(value: &RunBinding) -> wire::RunCoordinate {
    wire::RunCoordinate {
        id: *value.run().as_bytes(),
        expected_epoch: value.epoch().get(),
    }
}
pub(super) fn model(value: &ModelSelection) -> Result<body::ModelSelection, JournalError> {
    Ok(body::ModelSelection {
        provider: text(value.provider())?,
        model: text(value.model())?,
        pricing_policy: text(value.pricing())?,
    })
}
pub(super) fn target(
    target: &TargetBinding,
    authentication: &AuthenticationBinding,
) -> Result<body::TargetAccess, JournalError> {
    Ok(body::TargetAccess {
        target: body::TargetSelection {
            host_key: *target.host().as_bytes(),
            route: *target.route().as_bytes(),
            principal: text(target.principal())?,
            workspace: *target.workspace().as_bytes(),
        },
        authentication: body::AuthenticationSelection {
            public_key: *authentication.key().as_bytes(),
            credential_revision: *authentication.credential_revision().as_bytes(),
        },
    })
}
pub(super) fn session(value: &SessionReceipt) -> Result<body::SessionReference, JournalError> {
    Ok(body::SessionReference {
        id: *value.id().as_bytes(),
        epoch: value.epoch().get(),
        access: target(value.target(), value.authentication())?,
    })
}
pub(super) fn execution(
    value: &ExecutionAuthority,
) -> Result<body::ExecutionReference, JournalError> {
    Ok(body::ExecutionReference {
        run: run(value.run()),
        session: session(value.session())?,
        grant: body::GrantReference {
            id: *value.grant().as_bytes(),
            epoch: value.grant_epoch().get(),
        },
    })
}
pub(super) fn accepted(value: &CommandAcceptance) -> Result<AcceptedCommand, JournalError> {
    let kind = match value.kind() {
        CommandKind::SubmitPrompt => wire::CommandKind::SubmitPrompt,
        CommandKind::Stop => wire::CommandKind::Stop,
        CommandKind::ResolveApproval => wire::CommandKind::ResolveApproval,
        CommandKind::IssueGrant => wire::CommandKind::IssueGrant,
        CommandKind::RevokeGrant => wire::CommandKind::RevokeGrant,
        CommandKind::AuthenticationConsent => wire::CommandKind::AnswerSshConsent,
        CommandKind::DeleteChat => wire::CommandKind::DeleteChat,
        CommandKind::RevokeIssuer => wire::CommandKind::RevokeIssuerGrants,
        CommandKind::BeginSession => return Err(JournalError::UnsupportedFormat),
    };
    Ok(AcceptedCommand {
        id: *value.id().as_bytes(),
        kind,
        signed_message_digest: *value.digest().as_bytes(),
        issuer: wire::CommandIssuer {
            device_id: text(value.issuer().device())?,
            signing_key: *value.issuer().key().as_bytes(),
            access_token_digest: *value.issuer().invocation().as_bytes(),
        },
        expected_control_revision: value.expected_control().get(),
    })
}
pub(in crate::cloud) fn approval(
    value: &ApprovalAnswer,
) -> Result<body::ApprovalPayload, JournalError> {
    let choice = match value.choice() {
        ApprovalChoice::Approve => body::ApprovalChoice::Approve,
        ApprovalChoice::Deny => body::ApprovalChoice::Deny,
        ApprovalChoice::ReconcileKnown(fingerprint) => body::ApprovalChoice::ReconcileKnown {
            result_digest: *fingerprint.as_bytes(),
        },
    };
    Ok(body::ApprovalPayload {
        intent: *value.intent().as_bytes(),
        arguments_digest: *value.arguments().as_bytes(),
        execution: execution(value.authority())?,
        choice,
    })
}
pub(super) fn status(value: RunStatus) -> CloudRunStatus {
    match value {
        RunStatus::Queued => CloudRunStatus::Queued,
        RunStatus::Running => CloudRunStatus::Running,
        RunStatus::WaitingApproval => CloudRunStatus::WaitingApproval,
        RunStatus::NeedsGrant => CloudRunStatus::NeedsGrant,
        RunStatus::NeedsUnlock => CloudRunStatus::NeedsUnlock,
        RunStatus::NeedsProviderAccess => CloudRunStatus::NeedsProviderAccess,
        RunStatus::OutcomeUnknown => CloudRunStatus::OutcomeUnknown,
        RunStatus::Interrupted => CloudRunStatus::Interrupted,
        RunStatus::Stopped => CloudRunStatus::Stopped,
        RunStatus::Completed => CloudRunStatus::Completed,
    }
}
pub(super) fn usage(value: UsageObservation) -> CloudUsage {
    CloudUsage {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        total_tokens: value.total_tokens,
        cached_input_tokens: value.cached_input_tokens,
        thought_tokens: value.thought_tokens,
        cost_micros: value.cost_micros,
    }
}
