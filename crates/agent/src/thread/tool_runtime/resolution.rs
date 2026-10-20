//! Supplied workspace observations are finite inputs to turn context projection.
use crate::seat::Seat;
use gpui_kit::App;
use nocterm_workspace::{ConnectionSummary, TerminalEntry};
pub(in crate::thread) struct Resolution {
    terminals: Vec<TerminalEntry>,
    summaries: Vec<ConnectionSummary>,
}
impl Resolution {
    pub(in crate::thread) fn observe(seat: &Seat, cx: &App) -> Self {
        let Some(terminals) = seat.terminals(cx) else {
            return Self {
                terminals: Vec::new(),
                summaries: Vec::new(),
            };
        };
        Self {
            terminals,
            summaries: seat.connections(cx),
        }
    }
    pub(super) fn terminals(&self) -> &[TerminalEntry] {
        &self.terminals
    }
    pub(in crate::thread) fn summaries(&self) -> &[ConnectionSummary] {
        &self.summaries
    }
}

pub(in crate::thread) enum OpenSelection {
    Respond(serde_json::Value),
    Unknown,
    Closed,
    Reconnect(TerminalEntry, nocterm_workspace::ExpectedSession),
    Create(ConnectionSummary),
}
