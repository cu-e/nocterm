//! Decode only the bridge's known result shapes. Unknown fields stay visible.
use super::{Output, text};
use crate::panel::entries::tool_input::Section;
use serde::Deserialize;
use serde_json::Value;

const MAX_DECODE_BYTES: usize = 512 * 1024;
const MAX_WRAPPER_DEPTH: usize = 8;

fn payload(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if text.len() <= MAX_DECODE_BYTES => serde_json::from_str(text).ok(),
        _ => None,
    }
}

fn mcp_text(value: &Value) -> Option<(&str, bool)> {
    let object = value.as_object()?;
    if !object
        .keys()
        .all(|key| matches!(key.as_str(), "content" | "isError"))
    {
        return None;
    }
    let [item] = object.get("content")?.as_array()?.as_slice() else {
        return None;
    };
    let item = item.as_object()?;
    if item.len() != 2 || item.get("type")?.as_str()? != "text" {
        return None;
    }
    let error = match object.get("isError") {
        Some(Value::Bool(error)) => *error,
        None => false,
        _ => return None,
    };
    Some((item.get("text")?.as_str()?, error))
}

fn nested(value: &Value) -> Option<Value> {
    if let Some(payload) = payload(value) {
        return Some(payload);
    }
    let object = value.as_object()?;
    if object
        .keys()
        .all(|key| matches!(key.as_str(), "jsonrpc" | "id" | "result"))
        && object.get("jsonrpc").is_none_or(|version| version == "2.0")
        && let Some(result) = object.get("result")
    {
        return Some(result.clone());
    }
    let (message, error) = mcp_text(value)?;
    (!error).then(|| Value::String(message.into()))
}

fn normalized(value: &Value) -> Option<Value> {
    let mut value = value.clone();
    for _ in 0..MAX_WRAPPER_DEPTH {
        match nested(&value) {
            Some(next) => value = next,
            None => return Some(value),
        }
    }
    None
}

pub(super) fn equivalent(tool: Option<&str>, content: &Value, raw: &Value) -> bool {
    if content == raw {
        return true;
    }
    let Some(tool) = tool else {
        return false;
    };
    known(tool, content, 0).is_some()
        && known(tool, raw, 0).is_some()
        && normalized(content).is_some_and(|content| Some(content) == normalized(raw))
}

pub(super) fn known(tool: &str, value: &Value, depth: usize) -> Option<Output> {
    if depth >= MAX_WRAPPER_DEPTH {
        return None;
    }
    if let Some(nested) = nested(value) {
        return known(tool, &nested, depth + 1);
    }
    if let Some((message, true)) = mcp_text(value) {
        return Some(Output::plain("Error", message.into()));
    }
    let object = value.as_object()?;
    match tool {
        "exec_command" | "read_command" | "cancel_command" => command(value),
        "run_command" => observation(value),
        "read_terminal" => terminal(value),
        "send_input" if value == &serde_json::json!({"accepted":true}) => {
            Some(Output::plain("Tool output", "Input accepted".into()))
        }
        "list_terminals" if object.len() == 1 => Some(Output::plain(
            "Context",
            object.get("context")?.as_str()?.into(),
        )),
        "open_terminal" if object.len() == 1 && object.contains_key("terminal") => {
            Some(Output::plain("Terminal", text(object.get("terminal")?)))
        }
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    command_id: String,
    state: String,
    stdout: String,
    stderr: String,
    exit_status: Option<u32>,
    total_bytes: u64,
    truncated: bool,
    error: Option<String>,
}

fn command(value: &Value) -> Option<Output> {
    let value: Command = serde_json::from_value(value.clone()).ok()?;
    let state = match value.state.as_str() {
        "starting" => "Starting",
        "running" => "Running",
        "exited" => "Exited",
        "timed_out" => "Timed out",
        "cancelled" => "Cancelled",
        "failed" => "Failed",
        _ => return None,
    };
    if value.command_id.is_empty() || value.command_id.len() > 128 {
        return None;
    }
    let mut output = Output::plain("Standard output", value.stdout);
    output.sections.push(Section {
        label: "Standard error",
        text: value.stderr,
        language: None,
    });
    output.parameters = vec![
        format!("Process: {state}"),
        format!("Command handle: {}", value.command_id),
        value
            .exit_status
            .map(|status| format!("Exit status: {status}"))
            .unwrap_or_else(|| "Exit status: unknown".into()),
        format!("{} bytes received", value.total_bytes),
    ];
    if value.truncated {
        output.parameters.push("Output truncated".into());
    }
    if let Some(error) = value.error {
        output.sections.push(Section {
            label: "Error",
            text: error,
            language: None,
        });
    }
    Some(output)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    text: String,
    first_line: u64,
    next_line: u64,
    cursor_semantics: String,
    truncated: bool,
    alt_screen: bool,
}

fn terminal_output(value: Terminal) -> Option<Output> {
    if value.cursor_semantics != "inclusive snapshot; replace overlapping lines" {
        return None;
    }
    let mut output = Output::plain("Terminal output", value.text);
    output.parameters = vec![
        format!("Lines {}–{}", value.first_line, value.next_line),
        value.cursor_semantics,
    ];
    if value.truncated {
        output.parameters.push("Output truncated".into());
    }
    if value.alt_screen {
        output.parameters.push("Alternate screen".into());
    }
    Some(output)
}
fn terminal(value: &Value) -> Option<Output> {
    terminal_output(serde_json::from_value(value.clone()).ok()?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    text: String,
    first_line: u64,
    next_line: u64,
    cursor_semantics: String,
    truncated: bool,
    alt_screen: bool,
    completion: String,
    completed: bool,
    exit_status: Option<u32>,
}
fn observation(value: &Value) -> Option<Output> {
    let value: Observation = serde_json::from_value(value.clone()).ok()?;
    let status = match (value.completion.as_str(), value.completed) {
        ("prompt_returned", true) => "Prompt returned",
        ("output_idle", false) => "Observation ended: output idle; process completion unknown",
        ("timeout", false) => "Observation ended: timeout; process completion unknown",
        _ => return None,
    };
    if value.exit_status.is_some() {
        return None;
    }
    let mut output = terminal_output(Terminal {
        text: value.text,
        first_line: value.first_line,
        next_line: value.next_line,
        cursor_semantics: value.cursor_semantics,
        truncated: value.truncated,
        alt_screen: value.alt_screen,
    })?;
    output.parameters.push(status.into());
    output.parameters.push("Exit status: unknown".into());
    Some(output)
}
