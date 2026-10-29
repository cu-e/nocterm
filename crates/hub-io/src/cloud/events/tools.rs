//! Borrowed semantic arguments and admitted JSON presentation buffers.
use super::{encoding_error, values};
use nocterm_ai::{
    TerminalCall,
    tool_display::{ToolDisplay, ToolOutcome},
};
use nocterm_cloud_model::{
    journal::{JournalError, PublicationAllocation, PublicationWorkBudget},
    run::ToolIntent,
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::{
    events::{DisplayOutcome, ToolIntentPayload, ToolPresentation},
    tool_arguments::ToolArguments,
};
use serde::Serialize;

pub(super) struct JsonText {
    text: String,
    _allocation: AllocationLease,
}
impl JsonText {
    pub(super) fn new(
        value: &impl Serialize,
        work: &dyn PublicationWorkBudget,
    ) -> Result<Self, JournalError> {
        let bytes = nocterm_ai::history::json_size(value).map_err(encoding_error)?;
        if bytes > nocterm_hub_model::cloud::events::MAX_EVENT_JSON_BYTES {
            return Err(JournalError::Capacity);
        }
        let allocation = work.try_reserve(PublicationAllocation::new(bytes.max(1), 1)?)?;
        let mut output = Vec::with_capacity(bytes);
        serde_json::to_writer(&mut output, value).map_err(encoding_error)?;
        if output.len() != bytes {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            text: String::from_utf8(output).map_err(encoding_error)?,
            _allocation: allocation,
        })
    }
    pub(super) fn text(&self) -> &str {
        &self.text
    }
}

pub(super) struct DisplayJson {
    arguments: JsonText,
    outcome: Option<JsonText>,
}
impl DisplayJson {
    pub(super) fn new(
        display: &ToolDisplay,
        work: &dyn PublicationWorkBudget,
    ) -> Result<Self, JournalError> {
        if !matches!(display.version, 1 | 2) {
            return Err(JournalError::Corrupt);
        }
        let outcome = match &display.outcome {
            Some(ToolOutcome::Ok(value)) => Some(JsonText::new(value, work)?),
            _ => None,
        };
        Ok(Self {
            arguments: JsonText::new(&display.arguments, work)?,
            outcome,
        })
    }
    pub(super) fn view<'a>(&'a self, display: &'a ToolDisplay) -> ToolPresentation<'a> {
        let outcome = match &display.outcome {
            None => None,
            Some(ToolOutcome::Pending) => Some(DisplayOutcome::Pending),
            Some(ToolOutcome::Err(error)) => Some(DisplayOutcome::Err(error)),
            Some(ToolOutcome::Ok(_)) => self
                .outcome
                .as_ref()
                .map(|value| DisplayOutcome::Ok(value.text())),
        };
        ToolPresentation {
            tool: &display.tool,
            arguments_json: self.arguments.text(),
            destination: display.destination.as_deref(),
            redacted: display.redacted,
            outcome,
        }
    }
}

pub(super) fn call_json(
    call: &TerminalCall,
    work: &dyn PublicationWorkBudget,
) -> Result<JsonText, JournalError> {
    match call {
        TerminalCall::ListTerminals => JsonText::new(&serde_json::json!({}), work),
        TerminalCall::OpenTerminal(value) => JsonText::new(value, work),
        TerminalCall::ReadTerminal(value) => JsonText::new(value, work),
        TerminalCall::SendInput(value) => JsonText::new(value, work),
        TerminalCall::RunCommand(value) => JsonText::new(value, work),
        TerminalCall::ExecCommand(value) => JsonText::new(value, work),
        TerminalCall::ReadCommand(value) => JsonText::new(value, work),
        TerminalCall::CancelCommand(value) => JsonText::new(value, work),
    }
}

/// The caller admits the borrowed vector/box and principal strings before this
/// mapping. The sole SDK encoder still validates digest, bounds and wire bytes.
pub(super) fn intent<'a>(
    value: &ToolIntent,
    call: &'a TerminalCall,
    display: ToolPresentation<'a>,
) -> Result<ToolIntentPayload<'a>, JournalError> {
    Ok(ToolIntentPayload {
        id: *value.id().as_bytes(),
        step: *value.step().as_bytes(),
        execution: values::execution(value.authority())?,
        arguments_digest: *value.arguments().as_bytes(),
        mutates: value.mutates(),
        arguments: arguments(call)?,
        display,
    })
}
pub(super) fn arguments(call: &TerminalCall) -> Result<ToolArguments<'_>, JournalError> {
    Ok(match call {
        TerminalCall::ListTerminals => ToolArguments::ListTerminals,
        TerminalCall::OpenTerminal(value) => ToolArguments::OpenTerminal {
            server_id: &value.server_id,
        },
        TerminalCall::ReadTerminal(value) => ToolArguments::ReadTerminal {
            terminal_id: &value.terminal_id,
            lines: value
                .lines
                .map(u64::try_from)
                .transpose()
                .map_err(encoding_error)?,
            since: value.since,
        },
        TerminalCall::SendInput(value) => ToolArguments::SendInput {
            terminal_id: &value.terminal_id,
            text: &value.text,
            press_enter: value.press_enter,
        },
        TerminalCall::RunCommand(value) => ToolArguments::RunCommand {
            terminal_id: &value.terminal_id,
            command: &value.command,
            timeout_ms: value.timeout_ms,
            idle_ms: value.idle_ms,
        },
        TerminalCall::ExecCommand(value) => ToolArguments::ExecCommand {
            terminal_id: &value.terminal_id,
            program: &value.program,
            args: value.args.iter().map(String::as_str).collect(),
            stdin: value.stdin.as_deref(),
            timeout_ms: value.timeout_ms,
            yield_ms: value.yield_ms,
        },
        TerminalCall::ReadCommand(value) => ToolArguments::ReadCommand {
            terminal_id: &value.terminal_id,
            command_id: &value.command_id,
            yield_ms: value.yield_ms,
        },
        TerminalCall::CancelCommand(value) => ToolArguments::CancelCommand {
            terminal_id: &value.terminal_id,
            command_id: &value.command_id,
        },
    })
}
