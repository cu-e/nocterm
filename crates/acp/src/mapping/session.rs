use super::{
    content::{content, extensions},
    controls::options,
    tools::{patch, record},
};
use agent_client_protocol_schema::{MaybeUndefined, v1 as wire};
use nocterm_ai::model::{
    commands::AvailableCommand,
    controls::Change,
    diagnostic::ModelError,
    ids::{ModeId, SessionId},
    plan::{Plan, PlanEntry, PlanPriority, PlanStatus},
    session::{SessionInfo, SessionNotice, SessionUpdate},
    usage::{ContextUsage, Cost},
};

fn plan(value: wire::Plan) -> Result<Plan, ModelError> {
    value
        .entries
        .into_iter()
        .map(|entry| {
            Ok(PlanEntry {
                content: entry.content,
                priority: match entry.priority {
                    wire::PlanEntryPriority::High => PlanPriority::High,
                    wire::PlanEntryPriority::Medium => PlanPriority::Medium,
                    wire::PlanEntryPriority::Low => PlanPriority::Low,
                    _ => return Err(ModelError::Unsupported("plan priority")),
                },
                status: match entry.status {
                    wire::PlanEntryStatus::Pending => PlanStatus::Pending,
                    wire::PlanEntryStatus::InProgress => PlanStatus::InProgress,
                    wire::PlanEntryStatus::Completed => PlanStatus::Completed,
                    _ => return Err(ModelError::Unsupported("plan status")),
                },
            })
        })
        .collect::<Result<_, _>>()
        .map(|entries| Plan { entries })
}
fn command(value: wire::AvailableCommand) -> AvailableCommand {
    AvailableCommand {
        name: value.name,
        description: value.description,
        hint: value.input.and_then(|input| match input {
            wire::AvailableCommandInput::Unstructured(input) => Some(input.hint),
            _ => None,
        }),
    }
}
fn title(value: MaybeUndefined<String>) -> Change<String> {
    match value {
        MaybeUndefined::Value(value) => Change::Set(value),
        MaybeUndefined::Null => Change::Clear,
        _ => Change::Keep,
    }
}
fn update(value: wire::SessionUpdate) -> Result<SessionUpdate, ModelError> {
    Ok(match value {
        wire::SessionUpdate::UserMessageChunk(chunk) => {
            SessionUpdate::UserMessageChunk(content(chunk.content)?)
        }
        wire::SessionUpdate::AgentMessageChunk(chunk) => {
            SessionUpdate::AgentMessageChunk(content(chunk.content)?)
        }
        wire::SessionUpdate::AgentThoughtChunk(chunk) => {
            SessionUpdate::AgentThoughtChunk(content(chunk.content)?)
        }
        wire::SessionUpdate::ToolCall(call) => SessionUpdate::ToolCall(Box::new(record(call)?)),
        wire::SessionUpdate::ToolCallUpdate(call) => {
            SessionUpdate::ToolCallUpdate(Box::new(patch(call)?))
        }
        wire::SessionUpdate::Plan(value) => SessionUpdate::Plan(plan(value)?),
        wire::SessionUpdate::AvailableCommandsUpdate(value) => {
            SessionUpdate::AvailableCommandsUpdate(
                value.available_commands.into_iter().map(command).collect(),
            )
        }
        wire::SessionUpdate::CurrentModeUpdate(value) => {
            SessionUpdate::CurrentModeUpdate(ModeId::new(value.current_mode_id.to_string()))
        }
        wire::SessionUpdate::ConfigOptionUpdate(value) => {
            SessionUpdate::ConfigOptionUpdate(options(value.config_options)?)
        }
        wire::SessionUpdate::SessionInfoUpdate(value) => {
            SessionUpdate::SessionInfoUpdate(SessionInfo {
                title: title(value.title),
                meta: extensions(value.meta),
            })
        }
        wire::SessionUpdate::UsageUpdate(value) => SessionUpdate::UsageUpdate(ContextUsage {
            used: value.used,
            size: value.size,
            cost: value.cost.map(|cost| Cost {
                amount: cost.amount,
                currency: cost.currency,
            }),
            meta: extensions(value.meta),
        }),
        _ => return Err(ModelError::Unsupported("session update")),
    })
}
/// Records the neutral model cannot hold are reported, not guessed at.
pub(crate) fn notice(value: wire::SessionNotification) -> Result<SessionNotice, ModelError> {
    Ok(SessionNotice {
        session: SessionId::new(value.session_id.to_string()),
        update: update(value.update)?,
    })
}
