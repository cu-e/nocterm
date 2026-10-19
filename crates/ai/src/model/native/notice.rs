//! Owned live notices keep the exclusive payload lease of their transcript value.
use super::NativeOwned;
use crate::model::{
    content::Content,
    notice::SessionUpdate,
    tools::{ToolPatch, ToolRecord},
};

pub fn user_chunk(owned: NativeOwned<Content>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::UserChunk(value), allocation)
}
pub fn agent_chunk(owned: NativeOwned<Content>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::AgentChunk(value), allocation)
}
pub fn thought_chunk(owned: NativeOwned<Content>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::ThoughtChunk(value), allocation)
}
pub fn tool_started(owned: NativeOwned<ToolRecord>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::ToolStarted(value), allocation)
}
pub fn tool_changed(owned: NativeOwned<ToolPatch>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::ToolChanged(value), allocation)
}
pub fn plan(owned: NativeOwned<Vec<crate::model::notice::PlanStep>>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Plan(value), allocation)
}
pub fn plan_step(
    content: NativeOwned<String>,
    priority: crate::model::notice::PlanPriority,
    status: crate::model::notice::PlanStatus,
) -> NativeOwned<crate::model::notice::PlanStep> {
    let (content, allocation) = content.into_parts();
    NativeOwned::from_parts(
        crate::model::notice::PlanStep {
            content,
            priority,
            status,
        },
        allocation,
    )
}

pub fn notice(
    session: NativeOwned<crate::model::ids::SessionId>,
    update: NativeOwned<SessionUpdate>,
) -> Result<
    NativeOwned<crate::model::notice::SessionNotice>,
    crate::model::json::deferred::JsonDecodeError,
> {
    let ((session, update), allocation) = session.join(update)?.into_parts();
    Ok(NativeOwned::from_parts(
        crate::model::notice::SessionNotice { session, update },
        allocation,
    ))
}
pub fn command(
    name: NativeOwned<String>,
    description: NativeOwned<String>,
    input_hint: NativeOwned<Option<String>>,
) -> Result<
    NativeOwned<crate::model::notice::AdvertisedCommand>,
    crate::model::json::deferred::JsonDecodeError,
> {
    let (((name, description), input_hint), allocation) =
        name.join(description)?.join(input_hint)?.into_parts();
    Ok(NativeOwned::from_parts(
        crate::model::notice::AdvertisedCommand {
            name,
            description,
            input_hint,
        },
        allocation,
    ))
}
pub fn commands(
    owned: NativeOwned<Vec<crate::model::notice::AdvertisedCommand>>,
) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Commands(value), allocation)
}
pub fn mode(owned: NativeOwned<crate::model::ids::ModeId>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Mode(value), allocation)
}
pub fn configuration(
    owned: NativeOwned<Vec<crate::model::controls::ConfigOption>>,
) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Configuration(value), allocation)
}
pub fn from_presence(
    value: NativeOwned<crate::model::controls::Presence<String>>,
) -> NativeOwned<crate::model::controls::Change<String>> {
    use crate::model::controls::{Change, Presence};
    let (value, allocation) = value.into_parts();
    let change = match value {
        Presence::Absent => Change::Keep,
        Presence::Null => Change::Clear,
        Presence::Value(value) => Change::Set(value),
    };
    NativeOwned::from_parts(change, allocation)
}
pub fn info(
    owned: NativeOwned<crate::model::controls::Change<String>>,
) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Info(value), allocation)
}
pub fn usage_notice(
    context: NativeOwned<crate::model::usage::ContextUsage>,
    limits: NativeOwned<Vec<crate::usage::RateLimit>>,
) -> Result<
    NativeOwned<crate::model::notice::UsageNotice>,
    crate::model::json::deferred::JsonDecodeError,
> {
    let ((context, rate_limits), allocation) = context.join(limits)?.into_parts();
    Ok(NativeOwned::from_parts(
        crate::model::notice::UsageNotice {
            context,
            rate_limits,
        },
        allocation,
    ))
}
pub fn usage(owned: NativeOwned<crate::model::notice::UsageNotice>) -> NativeOwned<SessionUpdate> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SessionUpdate::Usage(value), allocation)
}
