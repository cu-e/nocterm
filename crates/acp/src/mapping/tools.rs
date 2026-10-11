use super::{content, identity};
use agent_client_protocol_schema::v1 as wire;
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
fn output(value: wire::ToolCallContent) -> Result<ToolOutput, ModelError> {
    Ok(match value {
        wire::ToolCallContent::Content(value) => ToolOutput::Content {
            content: Box::new(content::inbound(value.content)?),
            extensions: content::extensions(value.meta)?,
        },
        wire::ToolCallContent::Diff(value) => ToolOutput::FileDiff(FileDiff {
            path: value.path,
            previous: value.old_text,
            replacement: value.new_text,
            extensions: content::extensions(value.meta)?,
        }),
        wire::ToolCallContent::Terminal(value) => ToolOutput::TerminalReference {
            id: ProviderTerminalId::new(value.terminal_id.to_string()),
            extensions: content::extensions(value.meta)?,
        },
        _ => return Err(ModelError::Unsupported("tool output")),
    })
}
fn location(value: wire::ToolCallLocation) -> Result<Location, ModelError> {
    Ok(Location {
        path: value.path,
        line: value.line,
        extensions: content::extensions(value.meta)?,
    })
}
fn live_metadata(
    mut value: Option<wire::Meta>,
) -> Result<Option<nocterm_ai::model::extensions::Extensions>, ModelError> {
    if let Some(value) = &mut value {
        value.remove(nocterm_ai::tool_display::META_KEY);
    }
    content::extensions(value)
}
pub(super) fn record(value: wire::ToolCall) -> Result<ToolRecord, ModelError> {
    let claims = identity::claims(
        value.name.as_deref(),
        Some(&value.title),
        value.raw_input.as_ref(),
    );
    Ok(ToolRecord {
        id: ToolId::new(value.tool_call_id.to_string()),
        title: value.title,
        name: value.name,
        kind: kind(value.kind)?,
        status: status(value.status)?,
        output: value
            .content
            .into_iter()
            .map(output)
            .collect::<Result<_, _>>()?,
        locations: value
            .locations
            .into_iter()
            .map(location)
            .collect::<Result<_, _>>()?,
        arguments: value.raw_input,
        result: value.raw_output,
        extensions: live_metadata(value.meta)?,
        claims,
        client_display: None,
    })
}
pub(super) fn patch(value: wire::ToolCallUpdate) -> Result<ToolPatch, ModelError> {
    let fields = value.fields;
    let claims = identity::claims(
        fields.name.as_deref(),
        fields.title.as_deref(),
        fields.raw_input.as_ref(),
    );
    Ok(ToolPatch {
        id: ToolId::new(value.tool_call_id.to_string()),
        changes: ToolChanges {
            kind: fields.kind.map(kind).transpose()?,
            status: fields.status.map(status).transpose()?,
            title: fields.title,
            name: fields.name,
            output: fields
                .content
                .map(|values| values.into_iter().map(output).collect::<Result<_, _>>())
                .transpose()?,
            locations: fields
                .locations
                .map(|values| values.into_iter().map(location).collect::<Result<_, _>>())
                .transpose()?,
            arguments: fields.raw_input,
            result: fields.raw_output,
        },
        extensions: live_metadata(value.meta)?,
        claims,
    })
}
