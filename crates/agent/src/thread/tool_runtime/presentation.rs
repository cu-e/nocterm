//! Terminal outcome framing receives an explicit redaction policy.
use super::CommandSnapshot;
pub(in crate::thread) fn text_payload(
    tail: nocterm_workspace_ports::TerminalText,
    redact: bool,
) -> (serde_json::Value, usize) {
    let text = if redact {
        nocterm_ai::redact::redact(&tail.text)
    } else {
        tail.text
    };
    let bytes = text.len();
    (
        serde_json::json!({"text":text,"first_line":tail.first_line,"next_line":tail.next_line,"cursor_semantics":"inclusive snapshot; replace overlapping lines","truncated":tail.truncated,"alt_screen":tail.alt_screen}),
        bytes,
    )
}
pub(in crate::thread) fn command_payload(
    id: &str,
    state: CommandSnapshot,
    redact: bool,
) -> serde_json::Value {
    let text = |text: String| {
        if redact {
            nocterm_ai::redact::redact(&text)
        } else {
            text
        }
    };
    serde_json::json!({
        "command_id": id, "state": state.state().as_str(),
        "stdout": text(String::from_utf8_lossy(state.stdout()).into_owned()),
        "stderr": text(state.exit().map(|exit| exit.stderr.clone()).unwrap_or_default()),
        "exit_status": state.exit().and_then(|exit| exit.status),
        "total_bytes": state.total_bytes(), "truncated": state.total_bytes() > state.stdout().len() as u64,
        "error": state.error().map(nocterm_ai::redact::redact),
    })
}
