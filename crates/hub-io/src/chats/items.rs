//! Chat items of the hub protocol as the neutral session updates a chat shows.

use nocterm_ai::model::{
    content::Content,
    controls::{ConfigCategory, ConfigChoice, ConfigOption},
    ids::ToolId,
    session::SessionUpdate,
    tools::{ToolChanges, ToolKind, ToolPatch, ToolRecord, ToolStatus},
    usage::TokenTotals,
};
use nocterm_hub_proto::v2;

/// The configuration option that chooses a chat's model among its agent's.
pub(super) const MODEL: &str = "model";
/// The configuration option that chooses whether changing tools wait for an approval.
pub(super) const APPROVAL: &str = "approval";
const AUTO: &str = "auto";
const ASK: &str = "ask";

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

/// The options a hub chat offers: its model among the agent's models, and its approval mode.
pub(super) fn options(models: &[String], model: &str, approval: i32) -> Vec<ConfigOption> {
    let mut choices: Vec<ConfigChoice> = models
        .iter()
        .map(|model| ConfigChoice::new(model.clone(), model.clone()))
        .collect();
    if !model.is_empty() && !models.iter().any(|known| known == model) {
        choices.insert(0, ConfigChoice::new(model.to_owned(), model.to_owned()));
    }
    let ask = v2::ApprovalMode::try_from(approval) == Ok(v2::ApprovalMode::Ask);
    vec![
        ConfigOption::select(MODEL, "Model", model.to_owned(), choices)
            .category(ConfigCategory::Model),
        ConfigOption::select(
            APPROVAL,
            "Approval",
            if ask { ASK } else { AUTO },
            vec![
                ConfigChoice::new(AUTO, "Auto"),
                ConfigChoice::new(ASK, "Ask before changes"),
            ],
        )
        .category(ConfigCategory::Mode),
    ]
}

/// The approval mode a choice of the approval option stands for.
pub(super) fn approval_of(choice: &str) -> Option<v2::ApprovalMode> {
    match choice {
        AUTO => Some(v2::ApprovalMode::Auto),
        ASK => Some(v2::ApprovalMode::Ask),
        _ => None,
    }
}
