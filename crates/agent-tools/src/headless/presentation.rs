//! Bounded presentation uses the accepted native destination and actual observations.
use nocterm_ai::tools::output::{CommandReport, TerminalReport};
use nocterm_session::command_jobs::CommandSnapshot;
use serde_json::{Value, json};

pub(super) fn text(tail: nocterm_vt::TextTail) -> Value {
    json!(terminal(tail))
}
pub(super) fn terminal(tail: nocterm_vt::TextTail) -> TerminalReport {
    TerminalReport::new(
        tail.text,
        tail.first_line,
        tail.resume_line,
        tail.truncated,
        tail.alt_screen,
    )
}
pub(super) fn command(id: &str, snapshot: &CommandSnapshot) -> Value {
    json!(CommandReport::new(id, snapshot, false))
}
