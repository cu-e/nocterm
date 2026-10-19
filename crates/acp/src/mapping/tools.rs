use super::content::{content, extensions};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    diagnostic::ModelError,
    ids::{ProviderTerminalId, ToolId},
    tools::*,
};

fn kind(value: wire::ToolKind) -> ToolKind {
    match value {
        wire::ToolKind::Read => ToolKind::Read,
        wire::ToolKind::Edit => ToolKind::Edit,
        wire::ToolKind::Delete => ToolKind::Delete,
        wire::ToolKind::Move => ToolKind::Move,
        wire::ToolKind::Search => ToolKind::Search,
        wire::ToolKind::Execute => ToolKind::Execute,
        wire::ToolKind::Think => ToolKind::Think,
        wire::ToolKind::Fetch => ToolKind::Fetch,
        wire::ToolKind::SwitchMode => ToolKind::ChangeMode,
        // The saved-file grammar maps every unrecognized kind to `other`.
        _ => ToolKind::Other,
    }
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
fn output(value: wire::ToolCallContent) -> Result<ToolContent, ModelError> {
    Ok(match value {
        wire::ToolCallContent::Content(value) => ToolContent::Content(Box::new(ContentOutput {
            content: content(value.content)?,
            meta: extensions(value.meta),
        })),
        wire::ToolCallContent::Diff(value) => ToolContent::Diff(FileDiff {
            path: value.path,
            old_text: value.old_text,
            new_text: value.new_text,
            meta: extensions(value.meta),
        }),
        wire::ToolCallContent::Terminal(value) => ToolContent::Terminal(TerminalReference {
            terminal_id: ProviderTerminalId::new(value.terminal_id.to_string()),
            meta: extensions(value.meta),
        }),
        _ => return Err(ModelError::Unsupported("tool content")),
    })
}
fn outputs(values: Vec<wire::ToolCallContent>) -> Result<Vec<ToolContent>, ModelError> {
    values.into_iter().map(output).collect()
}
fn location(value: wire::ToolCallLocation) -> Location {
    Location {
        path: value.path,
        line: value.line,
        meta: extensions(value.meta),
    }
}
pub(super) fn record(value: wire::ToolCall) -> Result<ToolRecord, ModelError> {
    Ok(ToolRecord {
        id: ToolId::new(value.tool_call_id.to_string()),
        title: value.title,
        name: value.name,
        kind: kind(value.kind),
        status: status(value.status)?,
        content: outputs(value.content)?,
        locations: value.locations.into_iter().map(location).collect(),
        raw_input: value.raw_input,
        raw_output: value.raw_output,
        meta: extensions(value.meta),
    })
}
pub(super) fn patch(value: wire::ToolCallUpdate) -> Result<ToolPatch, ModelError> {
    let fields = value.fields;
    Ok(ToolPatch {
        id: ToolId::new(value.tool_call_id.to_string()),
        changes: ToolChanges {
            kind: fields.kind.map(kind),
            status: fields.status.map(status).transpose()?,
            title: fields.title,
            name: fields.name,
            content: fields.content.map(outputs).transpose()?,
            locations: fields
                .locations
                .map(|values| values.into_iter().map(location).collect()),
            raw_input: fields.raw_input,
            raw_output: fields.raw_output,
        },
        meta: extensions(value.meta),
    })
}
