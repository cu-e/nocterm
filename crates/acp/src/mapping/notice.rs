//! Live notifications become semantic facts before bounded event delivery.
use super::{content, controls, tools, usage};
use agent_client_protocol_schema::{MaybeUndefined, v1 as wire};
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{
    controls::Change,
    diagnostic::ModelError,
    ids::{ModeId, SessionId},
    notice::*,
};
pub(crate) fn inbound(
    authority: &RetentionAuthority,
    value: wire::SessionNotification,
) -> Result<Option<SessionNotice>, ModelError> {
    let update = match value.update {
        wire::SessionUpdate::UserMessageChunk(v) => {
            SessionUpdate::UserChunk(content::inbound(authority, v.content)?)
        }
        wire::SessionUpdate::AgentMessageChunk(v) => {
            SessionUpdate::AgentChunk(content::inbound(authority, v.content)?)
        }
        wire::SessionUpdate::AgentThoughtChunk(v) => {
            SessionUpdate::ThoughtChunk(content::inbound(authority, v.content)?)
        }
        wire::SessionUpdate::ToolCall(v) => {
            SessionUpdate::ToolStarted(tools::record(authority, v)?)
        }
        wire::SessionUpdate::ToolCallUpdate(v) => {
            SessionUpdate::ToolChanged(tools::patch(authority, v)?)
        }
        wire::SessionUpdate::Plan(v) => {
            SessionUpdate::Plan(v.entries.into_iter().map(plan).collect::<Result<_, _>>()?)
        }
        wire::SessionUpdate::AvailableCommandsUpdate(v) => {
            SessionUpdate::Commands(v.available_commands.into_iter().map(command).collect())
        }
        wire::SessionUpdate::CurrentModeUpdate(v) => {
            SessionUpdate::Mode(ModeId::new(v.current_mode_id.0.as_ref().to_owned()))
        }
        wire::SessionUpdate::ConfigOptionUpdate(v) => SessionUpdate::Configuration(
            v.config_options
                .into_iter()
                .map(|value| controls::option(authority, value))
                .collect::<Result<_, _>>()?,
        ),
        wire::SessionUpdate::SessionInfoUpdate(v) => SessionUpdate::Info(match v.title {
            MaybeUndefined::Value(v) => Change::Set(v),
            MaybeUndefined::Null => Change::Clear,
            _ => Change::Keep,
        }),
        wire::SessionUpdate::UsageUpdate(v) => SessionUpdate::Usage(UsageNotice {
            context: usage::context(&v),
            rate_limits: usage::limits(&v),
        }),
        _ => return Ok(None),
    };
    Ok(Some(SessionNotice {
        session: SessionId::new(value.session_id.0.as_ref().to_owned()),
        update,
    }))
}
fn plan(v: wire::PlanEntry) -> Result<PlanStep, ModelError> {
    let priority = match v.priority {
        wire::PlanEntryPriority::High => PlanPriority::High,
        wire::PlanEntryPriority::Medium => PlanPriority::Medium,
        wire::PlanEntryPriority::Low => PlanPriority::Low,
        _ => return Err(ModelError::Unsupported("plan priority")),
    };
    let status = match v.status {
        wire::PlanEntryStatus::Pending => PlanStatus::Pending,
        wire::PlanEntryStatus::InProgress => PlanStatus::Running,
        wire::PlanEntryStatus::Completed => PlanStatus::Complete,
        _ => return Err(ModelError::Unsupported("plan status")),
    };
    Ok(PlanStep {
        content: v.content,
        priority,
        status,
    })
}
fn command(v: wire::AvailableCommand) -> AdvertisedCommand {
    let input_hint = v.input.and_then(|input| match input {
        wire::AvailableCommandInput::Unstructured(v) => Some(v.hint),
        _ => None,
    });
    AdvertisedCommand {
        name: v.name,
        description: v.description,
        input_hint,
    }
}
