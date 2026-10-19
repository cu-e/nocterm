//! Client-owned presentation of requested Nocterm operations. This is saved with
//! the transcript so a later profile rename or reconnect cannot rewrite history.
use crate::model::{diagnostic::ModelError, json::OwnedJson, retention::RetentionAuthority};
use serde_json::{Value, json};

pub use crate::model::tools::{ClientDisplay as ToolDisplay, DisplayOutcome as ToolOutcome};
use crate::{
    TerminalCall, ToolFailure,
    model::{controls::Presence, tools::ToolRecord},
};

pub const META_KEY: &str = "nocterm/tool-display";

impl ToolDisplay {
    pub fn new(
        authority: &RetentionAuthority,
        call: &TerminalCall,
        destination: Option<String>,
    ) -> Result<Self, ModelError> {
        let (tool, arguments) = request(call);
        Ok(Self {
            version: 2,
            tool: tool.into(),
            arguments: OwnedJson::from_untracked(authority, arguments)?,
            destination: destination.map_or(Presence::Null, Presence::Value),
            redacted: Presence::Value(false),
            outcome: Presence::Value(ToolOutcome::Pending),
        })
    }

    pub fn requested(
        authority: &RetentionAuthority,
        tool: String,
        arguments: Value,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            version: 2,
            tool,
            arguments: OwnedJson::from_untracked(authority, arguments)?,
            destination: Presence::Null,
            redacted: Presence::Value(false),
            outcome: Presence::Value(ToolOutcome::Pending),
        })
    }

    pub fn finish(&mut self, result: Result<Value, ToolFailure>) -> Result<(), ModelError> {
        let outcome = match result {
            Ok(value) => ToolOutcome::Succeeded(OwnedJson::from_untracked(
                self.arguments.authority(),
                value,
            )?),
            Err(error) => ToolOutcome::Failed(crate::redact::redact(error.message())),
        };
        self.outcome = serde_json::to_vec(&outcome)
            .ok()
            .filter(|bytes| bytes.len() <= 256 * 1024)
            .map_or(Presence::Absent, |_| Presence::Value(outcome));
        Ok(())
    }

    pub fn from_call(call: &ToolRecord) -> Option<&Self> {
        let display = call.client_display.as_deref()?;
        (display.version == 2 || (display.version == 1 && display.display_request().is_some()))
            .then_some(display)
    }

    pub fn from_managed(
        call: &crate::history::physical::ToolSlot,
    ) -> Option<
        crate::history::physical::PayloadGuard<'_, crate::model::shared_display::SharedDisplay>,
    > {
        let display = call.client_display()?;
        (display.version == 2 || (display.version == 1 && display.display_request().is_some()))
            .then_some(display)
    }

    /// Display data has already been validated before execution. Redaction can
    /// expand its text beyond execution limits, so only deserialize its shape.
    /// Never pass this request back to an executor.
    pub fn display_request(&self) -> Option<TerminalCall> {
        deserialize(&self.tool, self.arguments.as_value().clone())
    }

    pub fn redact(&mut self) -> Result<(), ModelError> {
        let authority = self.arguments.authority().clone();
        let mut arguments = self.arguments.as_value().clone();
        let changed = redact_values(&mut arguments);
        let arguments = OwnedJson::from_untracked(&authority, arguments)?;
        let outcome = match &self.outcome {
            Presence::Value(ToolOutcome::Succeeded(value)) => {
                let mut value = value.as_value().clone();
                redact_values(&mut value);
                Presence::Value(ToolOutcome::Succeeded(OwnedJson::from_untracked(
                    &authority, value,
                )?))
            }
            Presence::Value(ToolOutcome::Failed(error)) => {
                Presence::Value(ToolOutcome::Failed(crate::redact::redact(error)))
            }
            other => other.clone(),
        };
        // Publish only after both complete immutable replacements were constructed.
        self.arguments = arguments;
        self.outcome = outcome;
        if changed {
            self.redacted = Presence::Value(true);
        }
        Ok(())
    }

    pub fn attach(&self, call: &mut ToolRecord) {
        if call.client_display.as_deref() == Some(self) {
            return;
        }
        call.extensions = call
            .extensions
            .take()
            .map(|values| values.without_client_display());
        call.client_display = Some(crate::model::shared_display::SharedDisplay::from_untracked(
            self.clone(),
        ));
    }

    pub fn title(&self) -> String {
        format!(
            "{} · {}",
            self.destination.value().map_or("Nocterm", String::as_str),
            operation(&self.tool)
        )
    }
}

pub fn operation(tool: &str) -> &'static str {
    crate::tools::catalog::get(tool).map_or("Tool", |spec| spec.label)
}

mod identity;
pub use identity::{
    bridge_server_name, envelope, fallback_tool, raw_envelope, raw_envelope_managed,
    requested_arguments, requested_arguments_managed, requested_call, requested_call_managed,
};

pub fn header(call: &ToolRecord) -> String {
    let title = ToolDisplay::from_call(call)
        .map(|display| display.title())
        .or_else(|| fallback_tool(call).map(|tool| format!("Nocterm · {}", operation(tool))))
        .unwrap_or_else(|| call.title.clone());
    format!("{title} · {}", call.status.label())
}

pub fn managed_header(call: &crate::history::physical::ToolSlot) -> String {
    let title = ToolDisplay::from_managed(call)
        .map(|display| display.title())
        .or_else(|| {
            requested_arguments_managed(call)
                .map(|(tool, _)| format!("Nocterm · {}", operation(&tool)))
        })
        .unwrap_or_else(|| call.title().to_string());
    format!("{title} · {}", call.status().label())
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

fn redact_values(value: &mut Value) -> bool {
    match value {
        Value::String(text) => {
            let redacted = crate::redact::redact(text);
            let changed = &redacted != text;
            *text = redacted;
            changed
        }
        Value::Array(values) => values
            .iter_mut()
            .fold(false, |changed, value| changed | redact_values(value)),
        Value::Object(values) => values
            .values_mut()
            .fold(false, |changed, value| changed | redact_values(value)),
        _ => false,
    }
}

/// Incoming live facts cannot supply client-owned presentation. History import
/// does not call this transition, so inert historical metadata stays literal.
pub fn strip_provider_meta(update: &mut crate::model::notice::SessionUpdate) {
    let extensions = match update {
        crate::model::notice::SessionUpdate::ToolStarted(call) => {
            call.client_display = None;
            &mut call.extensions
        }
        crate::model::notice::SessionUpdate::ToolChanged(patch) => &mut patch.extensions,
        _ => return,
    };
    if let Some(value) = extensions.take() {
        *extensions = Some(value.without_client_display());
    }
}

#[cfg(test)]
mod tests;
