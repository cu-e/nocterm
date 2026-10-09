//! Client-owned presentation of requested Nocterm operations. This is saved with
//! the transcript so a later profile rename or reconnect cannot rewrite history.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{TerminalCall, acp};

pub const META_KEY: &str = "nocterm/tool-display";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ToolOutcome {
    Pending,
    Ok(Value),
    Err(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDisplay {
    pub version: u8,
    pub tool: String,
    pub arguments: Value,
    pub destination: Option<String>,
    #[serde(default)]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ToolOutcome>,
}

impl ToolDisplay {
    pub fn new(call: &TerminalCall, destination: Option<String>) -> Self {
        let (tool, arguments) = request(call);
        Self {
            version: 2,
            tool: tool.into(),
            arguments,
            destination,
            redacted: false,
            outcome: Some(ToolOutcome::Pending),
        }
    }

    pub fn requested(tool: String, arguments: Value) -> Self {
        Self {
            version: 2,
            tool,
            arguments,
            destination: None,
            redacted: false,
            outcome: Some(ToolOutcome::Pending),
        }
    }

    pub fn finish(&mut self, result: Result<Value, String>) {
        let outcome = match result {
            Ok(value) => ToolOutcome::Ok(value),
            Err(error) => ToolOutcome::Err(crate::redact::redact(&error)),
        };
        self.outcome = serde_json::to_vec(&outcome)
            .ok()
            .filter(|bytes| bytes.len() <= 256 * 1024)
            .map(|_| outcome);
    }

    pub fn from_call(call: &acp::ToolCall) -> Option<Self> {
        let value = call.meta.as_ref()?.get(META_KEY)?.clone();
        let display: Self = serde_json::from_value(value).ok()?;
        (display.version == 2 || (display.version == 1 && display.display_request().is_some()))
            .then_some(display)
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
        match &mut self.outcome {
            Some(ToolOutcome::Ok(value)) => redact_values(value),
            Some(ToolOutcome::Err(error)) => *error = crate::redact::redact(error),
            _ => {}
        }
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
    crate::tools::catalog::get(tool).map_or("Tool", |spec| spec.label)
}

mod identity;
pub use identity::{
    bridge_server_name, envelope, fallback_tool, raw_envelope, requested_arguments, requested_call,
};

pub fn header(call: &acp::ToolCall) -> String {
    let title = ToolDisplay::from_call(call)
        .map(|display| display.title())
        .or_else(|| fallback_tool(call).map(|tool| format!("Nocterm · {}", operation(tool))))
        .unwrap_or_else(|| call.title.clone());
    format!("{title} · {:?}", call.status)
}

pub fn request(call: &TerminalCall) -> (&'static str, Value) {
    let arguments = match call {
        TerminalCall::ListTerminals => json!({}),
        TerminalCall::ReadTerminal(v) => json!(v),
        TerminalCall::SendInput(v) => json!(v),
        TerminalCall::RunCommand(v) => json!(v),
        TerminalCall::OpenTerminal(v) => json!(v),
        TerminalCall::ExecCommand(v) => json!(v),
        TerminalCall::ReadCommand(v) => json!(v),
        TerminalCall::CancelCommand(v) => json!(v),
    };
    (call.name(), arguments)
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
    (crate::tools::catalog::get(tool)?.parse)(args)
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
