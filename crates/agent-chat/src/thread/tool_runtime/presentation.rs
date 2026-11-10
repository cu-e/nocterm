//! Terminal outcome framing receives an explicit redaction policy.
use super::CommandSnapshot;
use nocterm_ai::tools::output::{CommandReport, TerminalReport};
pub(in crate::thread) fn text_payload(
    tail: nocterm_workspace_ports::TerminalText,
    redact: bool,
) -> (serde_json::Value, usize) {
    let report = terminal_report(tail, redact);
    let bytes = report.text.len();
    (serde_json::json!(report), bytes)
}
pub(in crate::thread) fn terminal_report(
    tail: nocterm_workspace_ports::TerminalText,
    redact: bool,
) -> TerminalReport {
    let text = if redact {
        nocterm_ai::redact::redact(&tail.text)
    } else {
        tail.text
    };
    TerminalReport::new(
        text,
        tail.first_line,
        tail.next_line,
        tail.truncated,
        tail.alt_screen,
    )
}
pub(in crate::thread) fn command_payload(
    id: &str,
    state: CommandSnapshot,
    redact: bool,
) -> serde_json::Value {
    serde_json::json!(CommandReport::new(id, &state, redact))
}
/// The agent-facing report of one transfer.
pub(in crate::thread) fn transfer_payload(
    progress: &nocterm_transfers::Progress,
    redact: bool,
) -> serde_json::Value {
    nocterm_ai::tools::transfer::report(progress, redact)
}
