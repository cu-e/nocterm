//! Historical provider wrappers are presentation hints, independent of execution validation.
use super::Output;
use serde_json::Value;
mod payload;
mod unwrap;
pub(super) use unwrap::truncated_preview;

pub(super) fn equivalent(tool: Option<&str>, content: &Value, raw: &Value) -> bool {
    content == raw
        || (tool.is_some()
            && unwrap::normalized(tool, content, 0)
                .is_some_and(|content| Some(content) == unwrap::normalized(tool, raw, 0)))
}

pub(super) fn known(tool: &str, value: &Value, depth: usize) -> Option<Output> {
    if let Some(output) = payload::known(tool, value) {
        return Some(output);
    }
    let Some(parts) = unwrap::normalized(Some(tool), value, depth) else {
        return Some(Output::plain(
            "Tool output",
            "Tool result exceeds display limits".into(),
        ));
    };
    let mut output = Output {
        sections: Vec::new(),
        parameters: Vec::new(),
    };
    for part in parts {
        output.append(match part {
            unwrap::Part::Payload(value) => bridge(tool, &value),
            unwrap::Part::Error(error) => Output::plain("Error", error),
        });
    }
    (!output.sections.is_empty()).then_some(output)
}

pub(super) fn bridge(tool: &str, value: &Value) -> Output {
    payload::known(tool, value).unwrap_or_else(|| Output::plain("Tool output", readable(value)))
}

/// Key/value text also handles future payloads without exposing provider JSON.
pub(in crate::panel::entries) fn readable(value: &Value) -> String {
    readable_at(value, 0)
}

fn readable_at(value: &Value, depth: usize) -> String {
    if depth >= 16 {
        return "…".into();
    }
    match value {
        Value::String(text) => text.clone(),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| format!("{key}: {}", readable_at(value, depth + 1)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Array(values) => values
            .iter()
            .map(|value| readable_at(value, depth + 1))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Null => "None".into(),
        value => value.to_string(),
    }
}
