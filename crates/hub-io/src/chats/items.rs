//! Chat items of the hub protocol as the neutral session updates a chat shows.

use nocterm_ai::model::{
    content::Content,
    ids::ToolId,
    session::SessionUpdate,
    tools::{ToolChanges, ToolKind, ToolPatch, ToolRecord, ToolStatus},
    usage::TokenTotals,
};
use nocterm_hub_proto::v2;

/// The prompt of a person's message: its text, and the names of its attachments.
pub(super) fn prompt(message: &v2::UserMessage) -> Vec<Content> {
    let mut content = Vec::with_capacity(1 + message.attachments.len());
    if !message.text.is_empty() {
        content.push(Content::text(message.text.clone()));
    }
    content.extend(
        message
            .attachments
            .iter()
            .map(|attachment| Content::text(format!("[{}]", attachment.name))),
    );
    content
}

fn kind_of(call: &v2::ToolCall) -> ToolKind {
    match call.tool.as_str() {
        "server.run" => ToolKind::Execute,
        "server.read_file" => ToolKind::Read,
        "server.write_file" => ToolKind::Edit,
        "server.send_file" => ToolKind::Fetch,
        _ if call.read_only => ToolKind::Read,
        _ => ToolKind::Other,
    }
}

pub(super) fn tool_call(call: &v2::ToolCall) -> SessionUpdate {
    let arguments = serde_json::from_str(&call.arguments_json).ok();
    let title = if call.title.is_empty() {
        call.tool.clone()
    } else {
        call.title.clone()
    };
    SessionUpdate::ToolCall(Box::new(
        ToolRecord::new(ToolId::new(call.call_id.clone()), title)
            .name(call.tool.clone())
            .kind(kind_of(call))
            .status(ToolStatus::Running)
            .raw_input(arguments),
    ))
}

pub(super) fn tool_result(result: &v2::ToolResult) -> SessionUpdate {
    let mut text = String::new();
    for part in &result.content {
        match &part.part {
            Some(v2::content_part::Part::Text(value)) => text.push_str(value),
            Some(v2::content_part::Part::File(file)) => {
                text.push_str(&format!("\n[{}]", file.name));
            }
            None => {}
        }
    }
    if result.truncated {
        text.push_str("\n… (output cut to the hub's limit)");
    }
    if result.unknown_outcome {
        text.push_str(
            "\nThe run was interrupted while this ran; whether it took effect is unknown.",
        );
    }
    let failed = result.is_error || result.unknown_outcome;
    let output = serde_json::json!({
        "exit_code": result.exit_code,
        "truncated": result.truncated,
    });
    SessionUpdate::ToolCallUpdate(Box::new(ToolPatch::new(
        ToolId::new(result.call_id.clone()),
        ToolChanges::default()
            .status(if failed {
                ToolStatus::Failed
            } else {
                ToolStatus::Succeeded
            })
            .content(vec![text.trim_start().to_owned().into()])
            .raw_output(output),
    )))
}

/// What the end of a run adds to the transcript: its error, when it has one.
pub(super) fn run_error(ended: &v2::RunEnded) -> Option<SessionUpdate> {
    (!ended.error.is_empty())
        .then(|| SessionUpdate::AgentMessageChunk(Content::text(format!("\n\n⚠ {}", ended.error))))
}

pub(super) fn usage(ended: &v2::RunEnded) -> Option<TokenTotals> {
    ended.usage.as_ref().map(|usage| {
        TokenTotals::new(
            usage.input_tokens + usage.output_tokens,
            usage.input_tokens,
            usage.output_tokens,
        )
    })
}
