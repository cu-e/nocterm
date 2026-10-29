//! Bounded presentation uses the accepted native destination and actual observations.
use nocterm_session::command_jobs::CommandSnapshot;
use serde_json::{Value, json};

pub(super) fn text(tail: nocterm_vt::TextTail) -> Value {
    json!({
        "text": tail.text, "first_line": tail.first_line,
        "next_line": tail.resume_line,
        "cursor_semantics": "inclusive snapshot; replace overlapping lines",
        "truncated": tail.truncated, "alt_screen": tail.alt_screen,
    })
}
pub(super) fn command(id: &str, snapshot: &CommandSnapshot) -> Value {
    json!({
        "command_id": id, "state": snapshot.state().as_str(),
        "stdout": String::from_utf8_lossy(snapshot.stdout()),
        "stderr": snapshot.exit().map(|exit| exit.stderr.clone()).unwrap_or_default(),
        "exit_status": snapshot.exit().and_then(|exit| exit.status),
        "total_bytes": snapshot.total_bytes(),
        "truncated": snapshot.total_bytes() > snapshot.stdout().len() as u64,
        "error": snapshot.error().map(|error| nocterm_ai::redact::redact(&error.to_string())),
    })
}
