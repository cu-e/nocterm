//! Historical tool conversion moves payloads; restored rows have no live claims.
use super::tools_decode::{Call, Output};
use super::{content, display};
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
fn output(authority: &RetentionAuthority, value: Output) -> Result<ToolOutput, ModelError> {
    Ok(match value {
        Output::Content(value) => ToolOutput::Content {
            content: Box::new(value.content.into_owned(authority)?),
            extensions: content::extensions(authority, value.meta)?,
        },
        Output::Diff(value) => ToolOutput::FileDiff(FileDiff {
            path: value.path,
            previous: value.old_text,
            replacement: value.new_text,
            extensions: content::extensions(authority, value.meta)?,
        }),
        Output::Terminal(value) => ToolOutput::TerminalReference {
            id: ProviderTerminalId::supplied(authority, value.terminal_id),
            extensions: content::extensions(authority, value.meta)?,
        },
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
pub(super) fn moving_ordered(
    authority: &RetentionAuthority,
    mut value: Call,
) -> Result<(ToolRecord, Option<super::order::RowOrder>), ModelError> {
    let original = value.meta.as_ref().and_then(|meta| {
        let slot = meta.keys().position(|key| key == "nocterm/tool-display")?;
        Some((
            slot,
            super::order::FieldOrder::capture(meta.get("nocterm/tool-display")?.as_object()?),
        ))
    });
    let literal = super::identity::literal(
        value.name.as_deref(),
        Some(&value.title),
        value.raw_input.as_ref(),
    );
    let client_display = match value.meta.as_mut() {
        Some(meta) => display::take(authority, meta)?,
        None => None,
    }
    .map(nocterm_ai::model::shared_display::SharedDisplay::from_untracked);
    let row = ToolRecord {
        id: ToolId::supplied(authority, value.tool_call_id),
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
        extensions: content::extensions(authority, value.meta)?,
        // Historical literal fields survive above, but cannot mint registration
        // authority or duplicate unbounded substrings into a claim sidecar.
        claims: BridgeClaims::default(),
        literal,
        client_display,
    };
    let order = original.and_then(|(slot, fields)| super::order::capture(&row, slot, fields));
    Ok((row, order))
}
