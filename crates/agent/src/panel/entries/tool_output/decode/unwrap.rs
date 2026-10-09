//! Bounded provider envelope removal; the entire Hermes fence must be complete.
use serde_json::Value;

const MAX_DECODE_BYTES: usize = 512 * 1024;
const MAX_WRAPPER_DEPTH: usize = 8;

#[derive(PartialEq)]
pub(super) enum Part {
    Payload(Value),
    Error(String),
}

pub(super) fn normalized(tool: Option<&str>, value: &Value, depth: usize) -> Option<Vec<Part>> {
    if serde_json::to_vec(value).ok()?.len() > MAX_DECODE_BYTES {
        return None;
    }
    let mut parts = Vec::new();
    descend(tool, value, depth, false, &mut parts)?;
    Some(parts)
}

fn descend(
    tool: Option<&str>,
    value: &Value,
    depth: usize,
    error: bool,
    parts: &mut Vec<Part>,
) -> Option<()> {
    if depth >= MAX_WRAPPER_DEPTH {
        return None;
    }
    if let Some(text) = value.as_str() {
        if let Some(result) = fenced(text) {
            return descend(tool, &Value::String(result.into()), depth + 1, error, parts);
        }
        if text.len() <= MAX_DECODE_BYTES
            && let Ok(next) = serde_json::from_str::<Value>(text)
        {
            return descend(tool, &next, depth + 1, error, parts);
        }
    }
    if let Some(values) = value.as_array() {
        for value in values {
            descend(tool, value, depth + 1, error, parts)?;
        }
        return Some(());
    }
    if let Some(object) = value.as_object()
        && !tool.is_some_and(|tool| super::payload::recognizes(tool, value))
    {
        let error = error
            || object
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if let Some(message) = provider_error(value) {
            parts.push(Part::Error(message));
            return Some(());
        }
        if let Some(result) = object.get("result") {
            return descend(tool, result, depth + 1, error, parts);
        }
        if let Some(content) = object.get("content").and_then(Value::as_array) {
            if content.is_empty() && error {
                parts.push(Part::Error("Tool request failed".into()));
            }
            for block in content {
                descend(tool, block, depth + 1, error, parts)?;
            }
            return Some(());
        }
        if object.get("type").and_then(Value::as_str) == Some("text")
            && let Some(text) = object.get("text").and_then(Value::as_str)
        {
            return descend(tool, &Value::String(text.into()), depth + 1, error, parts);
        }
        if object.get("isError").and_then(Value::as_bool) == Some(true) {
            parts.push(Part::Error("Tool request failed".into()));
            return Some(());
        }
    }
    parts.push(if error {
        Part::Error(super::readable(value))
    } else {
        Part::Payload(value.clone())
    });
    Some(())
}

fn provider_error(value: &Value) -> Option<String> {
    let error = value
        .get("error")?
        .as_str()
        .or_else(|| value.get("error")?.get("message")?.as_str())?;
    Some(error.into())
}

/// Only a whole-string fence with one guidance line is a provider wrapper.
fn fenced(text: &str) -> Option<&str> {
    let (source, rest) = text
        .strip_prefix("<untrusted_tool_result source=\"")?
        .split_once("\">\n")?;
    let (guidance, result) = rest.split_once("\n\n")?;
    if source.contains(['"', '\n']) || guidance.contains('\n') {
        return None;
    }
    result.strip_suffix("\n</untrusted_tool_result>")
}
