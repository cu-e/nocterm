//! Client-owned presentation of validated Nocterm requests. This is saved with
//! the transcript so a later profile rename or reconnect cannot rewrite history.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{TerminalCall, acp};

pub const META_KEY: &str = "nocterm/tool-display";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDisplay {
    pub version: u8,
    pub tool: String,
    pub arguments: Value,
    pub destination: Option<String>,
    #[serde(default)]
    pub redacted: bool,
}

impl ToolDisplay {
    pub fn new(call: &TerminalCall, destination: Option<String>) -> Self {
        let (tool, arguments) = request(call);
        Self {
            version: 1,
            tool: tool.into(),
            arguments,
            destination,
            redacted: false,
        }
    }

    pub fn from_call(call: &acp::ToolCall) -> Option<Self> {
        let value = call.meta.as_ref()?.get(META_KEY)?.clone();
        let display: Self = serde_json::from_value(value).ok()?;
        (display.version == 1 && display.display_request().is_some()).then_some(display)
    }

    /// Display data has already been validated before execution. Redaction can
    /// expand its text beyond execution limits, so only deserialize its shape.
    /// Never pass this request back to an executor.
    pub fn display_request(&self) -> Option<TerminalCall> {
        deserialize(&self.tool, self.arguments.clone())
    }

    pub fn redact(&mut self) {
        let original = self.arguments.clone();
        redact_values(&mut self.arguments);
        self.redacted |= self.arguments != original;
    }

    pub fn attach(&self, call: &mut acp::ToolCall) {
        if let Ok(value) = serde_json::to_value(self) {
            call.meta
                .get_or_insert_with(Default::default)
                .insert(META_KEY.into(), value);
        }
    }

    pub fn title(&self) -> String {
        format!(
            "{} · {}",
            self.destination.as_deref().unwrap_or("Nocterm"),
            operation(&self.tool)
        )
    }
}

pub fn operation(tool: &str) -> &'static str {
    match tool {
        "list_terminals" => "List terminals",
        "read_terminal" => "Read terminal",
        "send_input" => "Send input",
        "run_command" => "Run command",
        "open_terminal" => "Connect",
        "exec_command" => "Execute program",
        "read_command" => "Read command",
        "cancel_command" => "Cancel command",
        _ => "Tool",
    }
}

mod identity;
pub use identity::{bridge_server_name, envelope, fallback_tool, requested_call};

pub fn header(call: &acp::ToolCall) -> String {
    let title = ToolDisplay::from_call(call)
        .map(|display| display.title())
        .or_else(|| fallback_tool(call).map(|tool| format!("Nocterm · {}", operation(tool))))
        .unwrap_or_else(|| call.title.clone());
    format!("{title} · {:?}", call.status)
}

pub fn request(call: &TerminalCall) -> (&'static str, Value) {
    match call {
        TerminalCall::ListTerminals => ("list_terminals", json!({})),
        TerminalCall::ReadTerminal(v) => ("read_terminal", json!(v)),
        TerminalCall::SendInput(v) => ("send_input", json!(v)),
        TerminalCall::RunCommand(v) => ("run_command", json!(v)),
        TerminalCall::OpenTerminal(v) => ("open_terminal", json!(v)),
        TerminalCall::ExecCommand(v) => ("exec_command", json!(v)),
        TerminalCall::ReadCommand(v) => ("read_command", json!(v)),
        TerminalCall::CancelCommand(v) => ("cancel_command", json!(v)),
    }
}

/// A request the bridge would execute: well formed and within its limits.
pub fn parse(tool: &str, args: Value) -> Option<TerminalCall> {
    let call = deserialize(tool, args)?;
    call.validate().ok()?;
    Some(call)
}

/// A well-formed request, whether or not the bridge accepted it. Only for
/// showing what an agent asked for: a rejected request is still worth reading.
fn deserialize(tool: &str, args: Value) -> Option<TerminalCall> {
    let call = match tool {
        "list_terminals" if args == json!({}) => TerminalCall::ListTerminals,
        "read_terminal" => TerminalCall::ReadTerminal(serde_json::from_value(args).ok()?),
        "send_input" => TerminalCall::SendInput(serde_json::from_value(args).ok()?),
        "run_command" => TerminalCall::RunCommand(serde_json::from_value(args).ok()?),
        "open_terminal" => TerminalCall::OpenTerminal(serde_json::from_value(args).ok()?),
        "exec_command" => TerminalCall::ExecCommand(serde_json::from_value(args).ok()?),
        "read_command" => TerminalCall::ReadCommand(serde_json::from_value(args).ok()?),
        "cancel_command" => TerminalCall::CancelCommand(serde_json::from_value(args).ok()?),
        _ => return None,
    };
    Some(call)
}

fn redact_values(value: &mut Value) {
    match value {
        Value::String(text) => *text = crate::redact::redact(text),
        Value::Array(values) => values.iter_mut().for_each(redact_values),
        Value::Object(values) => values.values_mut().for_each(redact_values),
        _ => {}
    }
}

pub fn strip_provider_meta(update: &mut acp::SessionUpdate) {
    let meta = match update {
        acp::SessionUpdate::ToolCall(call) => &mut call.meta,
        acp::SessionUpdate::ToolCallUpdate(update) => &mut update.meta,
        _ => return,
    };
    if let Some(meta) = meta {
        meta.remove(META_KEY);
    }
}

#[cfg(test)]
mod tests;
