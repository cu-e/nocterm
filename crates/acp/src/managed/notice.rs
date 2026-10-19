//! Transcript-free notices and streamed content enter the original source ledger.
use super::{budget::SourceBudget, content, error::DecodeError, lists, value::Value};
use agent_client_protocol::managed::Span;
use nocterm_ai::model::{json::deferred::JsonDecodeError, native::NativeOwned, notice::*};
type Error = DecodeError<JsonDecodeError>;

#[cfg(test)]
pub(super) fn notice(
    span: &Span<SourceBudget>,
    budget: &SourceBudget,
) -> Result<Option<NativeOwned<SessionNotice>>, Error> {
    notice_with_models(span, budget, None)
}
pub(super) fn notice_with_models(
    span: &Span<SourceBudget>,
    budget: &SourceBudget,
    models: Option<&mut super::models::Models>,
) -> Result<Option<NativeOwned<SessionNotice>>, Error> {
    let root = Value::from_span(span, budget)?;
    let [session, update, _meta] = root.fields(["sessionId", "update", "_meta"])?;
    let session = Value::required(session, "sessionId")?.text()?;
    let update = Value::required(update, "update")?;
    let Some(update) = update_value(update, models, session.as_str())? else {
        return Ok(None);
    };
    Ok(Some(nocterm_ai::model::native::notice::notice(
        nocterm_ai::model::native::controls::session_id(session)?,
        update,
    )?))
}
fn update_value(
    value: Value<'_>,
    models: Option<&mut super::models::Models>,
    session: &str,
) -> Result<Option<NativeOwned<SessionUpdate>>, Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("session update object"));
    }
    let [kind] = value.fields(["sessionUpdate"])?;
    let kind = Value::required(kind, "sessionUpdate")?;
    for (name, role) in [
        ("user_message_chunk", 0),
        ("agent_message_chunk", 1),
        ("agent_thought_chunk", 2),
    ] {
        if kind.equals(name)? {
            let [content] = value.fields(["content"])?;
            let content = content::content(Value::required(content, "content")?)?;
            return Ok(Some(match role {
                0 => nocterm_ai::model::native::notice::user_chunk(content),
                1 => nocterm_ai::model::native::notice::agent_chunk(content),
                _ => nocterm_ai::model::native::notice::thought_chunk(content),
            }));
        }
    }
    if kind.equals("plan")? {
        let [entries] = value.fields(["entries"])?;
        return Ok(Some(nocterm_ai::model::native::notice::plan(
            lists::skipped(
                Value::required(entries, "entries")?,
                |value| Ok(lists::shape(plan_shape(value.clone()))?.is_some()),
                plan,
            )?,
        )));
    }
    if kind.equals("available_commands_update")? {
        let [commands] = value.fields(["availableCommands"])?;
        return Ok(Some(nocterm_ai::model::native::notice::commands(
            lists::skipped(
                Value::required(commands, "availableCommands")?,
                |value| Ok(lists::shape(command_shape(value.clone()))?.is_some()),
                command,
            )?,
        )));
    }
    control_update(&value, &kind, models, session)
}
/// Session controls, metadata, usage and tool notices.
fn control_update(
    value: &Value<'_>,
    kind: &Value<'_>,
    models: Option<&mut super::models::Models>,
    session: &str,
) -> Result<Option<NativeOwned<SessionUpdate>>, Error> {
    if kind.equals("current_mode_update")? {
        let [mode] = value.fields(["currentModeId"])?;
        return Ok(Some(nocterm_ai::model::native::notice::mode(
            nocterm_ai::model::native::controls::mode_id(
                Value::required(mode, "currentModeId")?.text()?,
            )?,
        )));
    }
    if kind.equals("session_info_update")? {
        let [title] = value.fields(["title"])?;
        let presence = match title {
            Some(title) if title.raw() == "null" => NativeOwned::<String>::null(value.ledger())?,
            Some(title) if title.raw().starts_with('"') => title.text()?.presence(),
            _ => NativeOwned::<String>::not_supplied(value.ledger())?,
        };
        return Ok(Some(nocterm_ai::model::native::notice::info(
            nocterm_ai::model::native::notice::from_presence(presence),
        )));
    }
    if kind.equals("config_option_update")? {
        let [options] = value.fields(["configOptions"])?;
        let options = super::controls::options(Value::required(options, "configOptions")?)?;
        let options = match models {
            Some(models) => models.retain(session, options)?,
            None => options,
        };
        return Ok(Some(nocterm_ai::model::native::notice::configuration(
            options,
        )));
    }
    if kind.equals("usage_update")? {
        return Ok(Some(nocterm_ai::model::native::notice::usage(
            super::usage::update(value.clone())?,
        )));
    }
    if kind.equals("tool_call")? {
        return Ok(Some(nocterm_ai::model::native::notice::tool_started(
            super::tools::record(value.clone())?,
        )));
    }
    if kind.equals("tool_call_update")? {
        return Ok(Some(nocterm_ai::model::native::notice::tool_changed(
            super::tools::patch(value.clone())?,
        )));
    }
    Ok(None)
}
fn plan_shape(value: Value<'_>) -> Result<(Value<'_>, PlanPriority, PlanStatus), Error> {
    let [content, priority, status, _meta] =
        value.fields(["content", "priority", "status", "_meta"])?;
    let content = Value::required(content, "plan.content")?;
    if !content.raw().starts_with('"') {
        return Err(DecodeError::Field("plan.content"));
    }
    let priority = Value::required(priority, "plan.priority")?;
    let priority = if priority.equals("high")? {
        PlanPriority::High
    } else if priority.equals("medium")? {
        PlanPriority::Medium
    } else if priority.equals("low")? {
        PlanPriority::Low
    } else {
        return Err(DecodeError::Field("plan.priority"));
    };
    let status = Value::required(status, "plan.status")?;
    let status = if status.equals("pending")? {
        PlanStatus::Pending
    } else if status.equals("in_progress")? {
        PlanStatus::Running
    } else if status.equals("completed")? {
        PlanStatus::Complete
    } else {
        return Err(DecodeError::Field("plan.status"));
    };
    Ok((content, priority, status))
}
fn plan(value: Value<'_>) -> Result<NativeOwned<PlanStep>, Error> {
    let (content, priority, status) = plan_shape(value)?;
    Ok(nocterm_ai::model::native::notice::plan_step(
        content.text()?,
        priority,
        status,
    ))
}
fn command_shape(value: Value<'_>) -> Result<(Value<'_>, Value<'_>, Option<Value<'_>>), Error> {
    let [name, description, input, _meta] =
        value.fields(["name", "description", "input", "_meta"])?;
    let name = Value::required(name, "command.name")?;
    let description = Value::required(description, "command.description")?;
    if !name.raw().starts_with('"') || !description.raw().starts_with('"') {
        return Err(DecodeError::Field("command text"));
    }
    let hint = match input {
        Some(input) => match lists::shape(input.fields(["hint", "_meta"]))? {
            Some([Some(hint), _]) if hint.raw().starts_with('"') => Some(hint),
            _ => None,
        },
        None => None,
    };
    Ok((name, description, hint))
}
fn command(value: Value<'_>) -> Result<NativeOwned<AdvertisedCommand>, Error> {
    let ledger = value.ledger();
    let (name, description, hint) = command_shape(value.clone())?;
    let hint = match hint {
        Some(hint) => NativeOwned::supplied(hint.text()?),
        None => NativeOwned::<String>::absent(ledger)?,
    };
    Ok(nocterm_ai::model::native::notice::command(
        name.text()?,
        description.text()?,
        hint,
    )?)
}
