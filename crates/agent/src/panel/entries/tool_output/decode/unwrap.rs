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

/// Hermes previews truncate before the closing fence. Only a matching complete
/// raw result proves that this exact provider preview can be replaced.
pub(in crate::panel::entries::tool_output) fn truncated_preview(
    content: &Value,
    raw: &Value,
) -> bool {
    let Some(preview) = content.as_str() else {
        return false;
    };
    let Some((prefix, total)) = preview.rsplit_once("\n... (") else {
        return false;
    };
    let Some(total) = total.strip_suffix(" chars total, truncated)") else {
        return false;
    };
    if !prefix.starts_with("<untrusted_tool_result source=\"") || fenced(prefix).is_some() {
        return false;
    }
    let Ok(total) = total.parse::<usize>() else {
        return false;
    };
    if !preview.ends_with(&format!("\n... ({total} chars total, truncated)")) {
        return false;
    }
    matches_fence(prefix, total, raw, 0)
}

fn matches_fence(prefix: &str, total: usize, value: &Value, depth: usize) -> bool {
    if depth >= MAX_WRAPPER_DEPTH {
        return false;
    }
    if let Some(text) = value.as_str() {
        if fenced(text).is_some() {
            return prefix.len() < text.len()
                && text.starts_with(prefix)
                && text.chars().count() == total;
        }
        return text.len() <= MAX_DECODE_BYTES
            && serde_json::from_str::<Value>(text)
                .ok()
                .is_some_and(|value| matches_fence(prefix, total, &value, depth + 1));
    }
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return false;
    }
    if let Some(result) = value.get("result") {
        return matches_fence(prefix, total, result, depth + 1);
    }
    if let Some([block]) = value
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        return matches_fence(prefix, total, block, depth + 1);
    }
    if value.get("type").and_then(Value::as_str) == Some("text")
        && let Some(text) = value.get("text")
    {
        return matches_fence(prefix, total, text, depth + 1);
    }
    false
}
