//! The device core's display values as the neutral conversation values of the desktop.
use nocterm_cloud_conversation_model::{
    Approval, CloudConversationError, CommandKey, CommandKind, CommandState, ConversationDelta,
    Freshness, IntentKey, ModelName, PendingCommand, Row, RowRole, Run, RunKey, RunStatus,
    SessionEnd, SessionRow, SessionState, ToolRow, ToolState, Usage,
};
use nocterm_cloud_device::{
    CloudDeviceError,
    flow::EffectFailure,
    reader,
    transcript::{self, TranscriptDelta},
};
use nocterm_core::error::SharedCause;
use nocterm_hub_model::cloud::{
    RunCoordinate,
    events::{CloudRunStatus, CloudUsage},
    payload::SessionFailure,
};

fn run_key(run: &RunCoordinate) -> RunKey {
    RunKey::from_bytes(run.id)
}

pub(crate) fn freshness(value: reader::Freshness) -> Freshness {
    match value {
        reader::Freshness::Current { witnessed_at_ms } => Freshness::Current { witnessed_at_ms },
        reader::Freshness::Unconfirmed {
            last_witnessed_at_ms,
        } => Freshness::Unconfirmed {
            last_witnessed_at_ms,
        },
        reader::Freshness::Withheld { witnessed_at_ms } => Freshness::Withheld { witnessed_at_ms },
    }
}

pub(crate) fn delta(delta: TranscriptDelta) -> ConversationDelta {
    ConversationDelta {
        reset: delta.reset,
        first_index: delta.first_index,
        changed_from: delta.changed_from,
        rows: delta.rows.iter().map(row).collect(),
        runs: delta.runs.map(|runs| runs.iter().map(run).collect()),
        approvals: delta.approvals.map(|approvals| {
            approvals
                .iter()
                .map(|request| Approval {
                    intent: IntentKey::from_bytes(request.intent()),
                    run: run_key(&request.run()),
                    expires_at_ms: request.expires_at_ms(),
                })
                .collect()
        }),
        pending: delta
            .pending
            .map(|pending| pending.iter().map(pending_command).collect()),
        control_revision: delta.control_revision,
        deleted: delta.deleted,
        omitted_earlier: delta.omitted_earlier,
    }
}

fn row(row: &transcript::TranscriptRow) -> Row {
    Row {
        index: row.index,
        role: match row.role {
            transcript::Role::User => RowRole::User,
            transcript::Role::Assistant => RowRole::Assistant,
            transcript::Role::Thought => RowRole::Thought,
            transcript::Role::Tool => RowRole::Tool,
            transcript::Role::Session => RowRole::Session,
        },
        run: row.run.as_ref().map(run_key),
        text: row.text.clone(),
        images: row.images,
        tool: row.tool.as_ref().map(|call| ToolRow {
            intent: IntentKey::from_bytes(call.intent),
            tool: call.tool.clone(),
            arguments_json: call.arguments_json.clone(),
            destination: call.destination.clone(),
            mutates: call.mutates,
            redacted: call.redacted,
            state: tool_state(call.status),
            output_json: call.output_json.clone(),
            outcome: call.outcome.clone(),
        }),
        session: row.session.as_ref().map(|call| SessionRow {
            attempt: call.attempt,
            principal: call.principal.clone(),
            host_key_sha256: call.host_key,
            ssh: call.ssh,
            terminal: call.terminal,
            state: session_state(call.status),
        }),
        at_ms: row.at_ms,
        updated_at_ms: row.updated_at_ms,
    }
}

fn tool_state(status: transcript::ToolStatus) -> ToolState {
    use transcript::ToolStatus as S;
    match status {
        S::Running => ToolState::Running,
        S::AwaitingApproval { expires_at_ms } => ToolState::AwaitingApproval { expires_at_ms },
        S::Approved => ToolState::Approved,
        S::Denied => ToolState::Denied,
        S::Succeeded => ToolState::Succeeded,
        S::Failed => ToolState::Failed,
        S::Unknown => ToolState::Unknown,
        S::Background => ToolState::Background,
    }
}

fn session_end(reason: SessionFailure) -> SessionEnd {
    match reason {
        SessionFailure::Denied => SessionEnd::Denied,
        SessionFailure::Expired => SessionEnd::Expired,
        SessionFailure::Cancelled => SessionEnd::Cancelled,
        SessionFailure::SourceChanged => SessionEnd::SourceChanged,
        SessionFailure::Unavailable => SessionEnd::Unavailable,
        SessionFailure::Unsupported => SessionEnd::Unsupported,
        SessionFailure::Capacity => SessionEnd::Capacity,
        SessionFailure::ConnectionLost => SessionEnd::ConnectionLost,
    }
}

fn session_state(status: transcript::SessionStatus) -> SessionState {
    use transcript::SessionStatus as S;
    match status {
        S::AwaitingConsent { expires_at_ms } => SessionState::AwaitingConsent { expires_at_ms },
        S::Established => SessionState::Established,
        S::Lost(reason) => SessionState::Lost(session_end(reason)),
        S::Refused(reason) => SessionState::Refused(session_end(reason)),
    }
}

pub(crate) fn run_status(status: CloudRunStatus) -> RunStatus {
    match status {
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

fn usage(usage: &CloudUsage) -> Usage {
    Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        thought_tokens: usage.thought_tokens,
        cost_micros: usage.cost_micros,
    }
}

fn run(state: &transcript::RunState) -> Run {
    Run {
        run: run_key(&state.run),
        status: run_status(state.status),
        usage: usage(&state.usage),
        model: ModelName {
            provider: state.model.provider.as_str().to_owned(),
            model: state.model.model.as_str().to_owned(),
        },
        prompt_command: CommandKey::from_bytes(state.prompt_command),
    }
}

pub(crate) fn command_state(state: transcript::CommandState) -> CommandState {
    use transcript::CommandState as S;
    match state {
        S::Sending => CommandState::Sending,
        S::Delivered => CommandState::Delivered,
        S::Unreachable => CommandState::Unreachable,
        S::Refused => CommandState::Refused,
        S::Accepted => CommandState::Accepted,
    }
}

fn pending_command(command: &transcript::PendingCommand) -> PendingCommand {
    PendingCommand {
        command: CommandKey::from_bytes(command.command_id),
        kind: match command.kind {
            transcript::PendingKind::Prompt => CommandKind::Prompt,
            transcript::PendingKind::Stop => CommandKind::Stop,
            transcript::PendingKind::Allow => CommandKind::Allow,
            transcript::PendingKind::Answer => CommandKind::Answer,
            transcript::PendingKind::Provider => CommandKind::Provider,
            transcript::PendingKind::Bind
            | transcript::PendingKind::Unbind
            | transcript::PendingKind::Certificate => CommandKind::Servers,
        },
        text: command.text.clone(),
        run: command.run.as_ref().map(run_key),
        state: command_state(command.state),
        at_ms: command.at_ms,
    }
}

/// A refusal of the core as the conversation's error; the cause is kept.
pub(crate) fn error(error: CloudDeviceError) -> CloudConversationError {
    match error {
        CloudDeviceError::NoRun => CloudConversationError::NoRun,
        CloudDeviceError::NoApproval => CloudConversationError::NoApproval,
        CloudDeviceError::NoModel => CloudConversationError::NoModel,
        CloudDeviceError::Unsupported => CloudConversationError::Unsupported,
        CloudDeviceError::Capacity => CloudConversationError::Capacity,
        CloudDeviceError::Ended => CloudConversationError::Ended,
        CloudDeviceError::Effect(EffectFailure::Unavailable | EffectFailure::Unknown) => {
            CloudConversationError::Unavailable
        }
        other => CloudConversationError::Refused(SharedCause::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_core_refusal_keeps_its_meaning_and_its_cause() {
        assert!(matches!(
            error(CloudDeviceError::NoModel),
            CloudConversationError::NoModel
        ));
        assert!(matches!(
            error(CloudDeviceError::Effect(EffectFailure::Unknown)),
            CloudConversationError::Unavailable
        ));
        let refused = error(CloudDeviceError::Rollback);
        assert!(matches!(refused, CloudConversationError::Refused(_)));
        let source = std::error::Error::source(&refused).expect("the core's cause");
        assert!(source.to_string().contains("behind"));
    }

    #[test]
    fn freshness_keeps_when_the_runner_last_answered() {
        assert_eq!(
            freshness(reader::Freshness::Unconfirmed {
                last_witnessed_at_ms: Some(4)
            }),
            Freshness::Unconfirmed {
                last_witnessed_at_ms: Some(4)
            }
        );
        assert!(freshness(reader::Freshness::Current { witnessed_at_ms: 1 }).is_current());
    }

    #[test]
    fn an_empty_delta_maps_to_an_empty_change() {
        let mapped = delta(TranscriptDelta::default());
        assert_eq!(mapped, ConversationDelta::default());
    }
}
