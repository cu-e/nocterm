//! Decode the known bridge payload independently of provider envelopes.
use super::super::Output;
use crate::panel::entries::tool_input::Section;
use serde::Deserialize;
use serde_json::Value;

/// Shape recognition protects payload fields from being mistaken for wrappers.
pub(super) fn recognizes(tool: &str, value: &Value) -> bool {
    match tool {
        "exec_command" | "read_command" | "cancel_command" => {
            serde_json::from_value::<Command>(value.clone()).is_ok()
        }
        "run_command" => serde_json::from_value::<Observation>(value.clone()).is_ok(),
        "read_terminal" => serde_json::from_value::<Terminal>(value.clone()).is_ok(),
        "list_terminals" => value.get("context").is_some_and(Value::is_string),
        "send_input" => value.get("accepted").is_some_and(Value::is_boolean),
        "open_terminal" => value.get("terminal").is_some_and(Value::is_object),
        _ => false,
    }
}

pub(super) fn known(tool: &str, value: &Value) -> Option<Output> {
    let object = value.as_object()?;
    let (mut output, fields) = match tool {
        "exec_command" | "read_command" | "cancel_command" => (
            command(value)?,
            &[
                "command_id",
                "state",
                "stdout",
                "stderr",
                "exit_status",
                "total_bytes",
                "truncated",
                "error",
            ][..],
        ),
        "run_command" => (
            observation(value)?,
            &[
                "text",
                "first_line",
                "next_line",
                "cursor_semantics",
                "truncated",
                "alt_screen",
                "completion",
                "completed",
                "exit_status",
            ][..],
        ),
        "read_terminal" => (
            terminal(value)?,
            &[
                "text",
                "first_line",
                "next_line",
                "cursor_semantics",
                "truncated",
                "alt_screen",
            ][..],
        ),
        "send_input" if object.get("accepted")?.as_bool()? => (
            Output::plain("Tool output", "Input accepted".into()),
            &["accepted"][..],
        ),
        "list_terminals" => (
            Output::plain("Context", object.get("context")?.as_str()?.into()),
            &["context"][..],
        ),
        "open_terminal" => (
            Output::plain(
                "Terminal",
                super::readable(&Value::Object(object.get("terminal")?.as_object()?.clone())),
            ),
            &["terminal"][..],
        ),
        _ => return None,
    };
    for (key, value) in object {
        if !fields.contains(&key.as_str()) {
            output
                .parameters
                .push(format!("{key}: {}", super::readable(value)));
        }
    }
    Some(output)
}

#[derive(Deserialize)]
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

pub(super) fn command(value: &Value) -> Option<Output> {
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
pub(super) fn terminal(value: &Value) -> Option<Output> {
    terminal_output(serde_json::from_value(value.clone()).ok()?)
}

#[derive(Deserialize)]
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
pub(super) fn observation(value: &Value) -> Option<Output> {
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
