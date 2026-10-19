//! Pure presentation diagnostics for composition-root regression tests.
use super::entries::{tool_input, tool_output};
use nocterm_ai::{model::tools::ToolRecord, tool_display};
use serde_json::Value;

/// Returns `(is_nocterm, [raw_header, raw_input, raw_output])` using the real formatters.
/// These presentation facts convey no execution authority.
pub fn tool_row_diagnostics(call: &ToolRecord) -> (bool, [bool; 3]) {
    (nocterm_row(call), raw_parts(call))
}

fn wrapper_text(text: &str) -> bool {
    if text.contains("<untrusted_tool_result") {
        return true;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| {
            [
                "result",
                "content",
                "jsonrpc",
                "isError",
                "structuredContent",
            ]
            .iter()
            .any(|key| object.contains_key(*key))
        })
}

fn raw_parts(call: &ToolRecord) -> [bool; 3] {
    let header = tool_display::header(call).starts_with("mcp");
    let input = tool_input::source(call).is_some_and(|input| {
        (input.label == "Tool input" && input.language == Some("json"))
            || wrapper_text(&input.text)
            || input
                .extra
                .iter()
                .any(|section| wrapper_text(&section.text))
            || input.parameters.iter().any(|text| wrapper_text(text))
    });
    let output = tool_output::source(call, false).is_some_and(|output| {
        output
            .sections
            .iter()
            .any(|section| wrapper_text(&section.text))
            || output.parameters.iter().any(|text| wrapper_text(text))
    });
    [header, input, output]
}

fn nocterm_row(call: &ToolRecord) -> bool {
    [Some(call.title.as_str()), call.name.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| {
            name.starts_with("mcp.nocterm-")
                || name.starts_with("mcp__nocterm-")
                || name.starts_with("mcp_nocterm_")
        })
        || call
            .arguments
            .as_ref()
            .map(|input| input.as_value())
            .and_then(|input| input.get("server"))
            .and_then(Value::as_str)
            .is_some_and(|server| server == "nocterm" || server.starts_with("nocterm-"))
        || tool_display::ToolDisplay::from_call(call).is_some()
}
