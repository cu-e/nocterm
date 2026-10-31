//! Only the existing terminal catalog and neutral text/usage cross the boundary.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use nocterm_ai::{model::content::Content, tools::catalog};
use nocterm_cloud_model::{
    engine::{EngineError, EnginePrompt, EngineToolCall, ToolSlot, UsageObservation},
    identity::NamespaceId,
};
use rig_agent::run::ExecCall;
use rig_core::{
    completion::{ToolDefinition, Usage},
    message::{ImageMediaType, Message, ToolName, UserContent},
};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) fn prompt(value: &EnginePrompt) -> Result<Message, EngineError> {
    let content = value
        .content()
        .iter()
        .map(|part| match part {
            Content::Text(text) => Ok(UserContent::text(&text.text)),
            Content::Image(image) if image.uri.is_none() => {
                let media = match image.mime_type.as_str() {
                    "image/png" => ImageMediaType::PNG,
                    "image/jpeg" => ImageMediaType::JPEG,
                    "image/webp" => ImageMediaType::WEBP,
                    "image/gif" => ImageMediaType::GIF,
                    _ => return Err(EngineError::UnsupportedContent),
                };
                let bytes = STANDARD
                    .decode(&image.data)
                    .map_err(|_| EngineError::UnsupportedContent)?;
                if ImageMediaType::sniff(&bytes) != Some(media.clone()) {
                    return Err(EngineError::UnsupportedContent);
                }
                Ok(UserContent::image_base64(
                    image.data.clone(),
                    Some(media),
                    None,
                ))
            }
            _ => Err(EngineError::UnsupportedContent),
        })
        .collect::<Result<_, _>>()?;
    Ok(Message::User { content })
}
/// The catalog tools the model is offered: all of them, or only the named ones.
/// A name outside the catalog never matches.
pub(crate) fn tools(
    allowed: Option<&BTreeSet<String>>,
) -> Result<Vec<Arc<ToolDefinition>>, EngineError> {
    catalog::TOOLS
        .iter()
        .filter(|spec| allowed.is_none_or(|names| names.contains(spec.name)))
        .map(|spec| {
            let name = ToolName::new(spec.name).map_err(|_| EngineError::InvalidStream)?;
            Ok(Arc::new(ToolDefinition::new(
                name,
                spec.description,
                (spec.schema)(),
            )))
        })
        .collect()
}
pub(crate) fn tool(slot: ToolSlot, call: &ExecCall) -> Result<EngineToolCall, EngineError> {
    let spec = catalog::get(call.name().as_str()).ok_or(EngineError::InvalidStream)?;
    let parsed = (spec.parse)(call.arguments()).ok_or(EngineError::InvalidStream)?;
    let provider_id = NamespaceId::new(call.id().wire()).ok();
    EngineToolCall::new(slot, provider_id, parsed)
}
pub(crate) fn usage(value: Usage) -> UsageObservation {
    UsageObservation {
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        total_tokens: value.total_tokens,
        cached_input_tokens: value.cached_input_tokens,
        thought_tokens: value.reasoning_tokens,
        // Rig can derive catalog pricing. It is not a provider invoice and must
        // never become a reported provider charge at this boundary.
        cost_micros: None,
    }
}

pub(crate) fn history(
    history: &nocterm_cloud_model::engine::EngineHistory,
) -> Result<Vec<Arc<Message>>, EngineError> {
    use nocterm_cloud_model::engine::EngineHistoryMessage;
    use rig_core::message::{AssistantContent, AssistantMessage, ToolCall, ToolFunction};
    let mut pending = std::collections::BTreeMap::new();
    let mut messages = Vec::new();
    for message in history.messages() {
        let message = match message {
            EngineHistoryMessage::User(user) => prompt(user)?,
            EngineHistoryMessage::Assistant(assistant) => {
                let mut content = Vec::new();
                if !assistant.text().is_empty() {
                    content.push(AssistantContent::text(assistant.text()));
                }
                for call in assistant.calls() {
                    let id = format!(
                        "nocterm_{}",
                        call.intent()
                            .as_bytes()
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    );
                    let name = ToolName::new(call.call().name())
                        .map_err(|_| EngineError::InvalidStream)?;
                    let wire = ToolCall::from_wire(
                        id,
                        ToolFunction::new(name, call.arguments().value().clone()),
                    );
                    pending.insert(call.intent(), wire.clone());
                    content.push(AssistantContent::ToolCall(wire));
                }
                Message::Assistant(AssistantMessage::new(content))
            }
            EngineHistoryMessage::ToolResult(answer) => {
                let call = pending
                    .remove(&answer.intent())
                    .ok_or(EngineError::InvalidStream)?;
                Message::User {
                    content: vec![UserContent::ToolResult(
                        call.answer(&result(answer.output(), answer.failed())),
                    )],
                }
            }
        };
        messages.push(Arc::new(message));
    }
    if !pending.is_empty() {
        return Err(EngineError::InvalidStream);
    }
    Ok(messages)
}
pub(crate) fn result(
    output: &nocterm_cloud_model::payload::BoundedJson,
    failed: bool,
) -> rig_core::tool::ToolResult {
    use rig_core::tool::{ToolExecutionError, ToolOutput, ToolResult};
    let output = ToolOutput::text(output.value().to_string());
    if failed {
        ToolResult::failed(ToolExecutionError::other("the host tool failed")).with_output(output)
    } else {
        ToolResult::success(output)
    }
}
