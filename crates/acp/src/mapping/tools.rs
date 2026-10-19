use super::{content, identity};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{
    diagnostic::ModelError,
    ids::{ProviderTerminalId, ToolId},
    tools::*,
};
fn kind(value: wire::ToolKind) -> Result<ToolKind, ModelError> {
    Ok(match value {
        wire::ToolKind::Read => ToolKind::Read,
        wire::ToolKind::Edit => ToolKind::Edit,
        wire::ToolKind::Delete => ToolKind::Delete,
        wire::ToolKind::Move => ToolKind::Move,
        wire::ToolKind::Search => ToolKind::Search,
        wire::ToolKind::Execute => ToolKind::Execute,
        wire::ToolKind::Think => ToolKind::Think,
        wire::ToolKind::Fetch => ToolKind::Fetch,
        wire::ToolKind::SwitchMode => ToolKind::ChangeMode,
        wire::ToolKind::Other => ToolKind::Other,
        _ => return Err(ModelError::Unsupported("tool kind")),
    })
}
fn status(value: wire::ToolCallStatus) -> Result<ToolStatus, ModelError> {
    Ok(match value {
        wire::ToolCallStatus::Pending => ToolStatus::Pending,
        wire::ToolCallStatus::InProgress => ToolStatus::Running,
        wire::ToolCallStatus::Completed => ToolStatus::Succeeded,
        wire::ToolCallStatus::Failed => ToolStatus::Failed,
        _ => return Err(ModelError::Unsupported("tool status")),
    })
}
fn output(
    authority: &RetentionAuthority,
    value: wire::ToolCallContent,
) -> Result<ToolOutput, ModelError> {
    Ok(match value {
        wire::ToolCallContent::Content(value) => ToolOutput::Content {
            content: Box::new(content::inbound(authority, value.content)?),
            extensions: content::extensions(authority, value.meta)?,
        },
        wire::ToolCallContent::Diff(value) => ToolOutput::FileDiff(FileDiff {
            path: value.path,
            previous: value.old_text,
            replacement: value.new_text,
            extensions: content::extensions(authority, value.meta)?,
        }),
        wire::ToolCallContent::Terminal(value) => ToolOutput::TerminalReference {
            id: ProviderTerminalId::supplied(authority, value.terminal_id.0.as_ref().to_owned()),
            extensions: content::extensions(authority, value.meta)?,
        },
        _ => return Err(ModelError::Unsupported("tool output")),
    })
}
fn location(
    authority: &RetentionAuthority,
    value: wire::ToolCallLocation,
) -> Result<Location, ModelError> {
    Ok(Location {
        path: value.path,
        line: value.line,
        extensions: content::extensions(authority, value.meta)?,
    })
}
fn live_metadata(
    authority: &RetentionAuthority,
    mut value: Option<wire::Meta>,
) -> Result<Option<nocterm_ai::model::extensions::Extensions>, ModelError> {
    if let Some(value) = &mut value {
        value.remove(nocterm_ai::tool_display::META_KEY);
    }
    content::extensions(authority, value)
}
pub(super) fn record(
    authority: &RetentionAuthority,
    value: wire::ToolCall,
) -> Result<ToolRecord, ModelError> {
    let literal = identity::literal(
        value.name.as_deref(),
        Some(&value.title),
        value.raw_input.as_ref(),
    );
    let claims = identity::claims(
        value.name.as_deref(),
        Some(&value.title),
        value.raw_input.as_ref(),
    );
    Ok(ToolRecord {
        id: ToolId::supplied(authority, value.tool_call_id.0.as_ref().to_owned()),
        title: value.title,
        name: value.name,
        kind: kind(value.kind)?,
        status: status(value.status)?,
        output: value
            .content
            .into_iter()
            .map(|value| output(authority, value))
            .collect::<Result<_, _>>()?,
        locations: value
            .locations
            .into_iter()
            .map(|value| location(authority, value))
            .collect::<Result<_, _>>()?,
        arguments: value
            .raw_input
            .map(|value| nocterm_ai::model::json::OwnedJson::from_untracked(authority, value))
            .transpose()?,
        result: value
            .raw_output
            .map(|value| nocterm_ai::model::json::OwnedJson::from_untracked(authority, value))
            .transpose()?,
        extensions: live_metadata(authority, value.meta)?,
        claims,
        literal,
        client_display: None,
    })
}
pub(super) fn patch(
    authority: &RetentionAuthority,
    value: wire::ToolCallUpdate,
) -> Result<ToolPatch, ModelError> {
    let fields = value.fields;
    let literal = identity::literal(
        fields.name.as_deref(),
        fields.title.as_deref(),
        fields.raw_input.as_ref(),
    );
    let claims = identity::claims(
        fields.name.as_deref(),
        fields.title.as_deref(),
        fields.raw_input.as_ref(),
    );
    Ok(ToolPatch {
        id: ToolId::supplied(authority, value.tool_call_id.0.as_ref().to_owned()),
        changes: ToolChanges {
            kind: fields.kind.map(kind).transpose()?,
            status: fields.status.map(status).transpose()?,
            title: fields.title,
            name: fields.name,
            output: fields
                .content
                .map(|values| {
                    values
                        .into_iter()
                        .map(|value| output(authority, value))
                        .collect::<Result<_, _>>()
                })
                .transpose()?,
            locations: fields
                .locations
                .map(|values| {
                    values
                        .into_iter()
                        .map(|value| location(authority, value))
                        .collect::<Result<_, _>>()
                })
                .transpose()?,
            arguments: fields
                .raw_input
                .map(|value| nocterm_ai::model::json::OwnedJson::from_untracked(authority, value))
                .transpose()?,
            result: fields
                .raw_output
                .map(|value| nocterm_ai::model::json::OwnedJson::from_untracked(authority, value))
                .transpose()?,
        },
        extensions: live_metadata(authority, value.meta)?,
        claims,
        literal,
    })
}
