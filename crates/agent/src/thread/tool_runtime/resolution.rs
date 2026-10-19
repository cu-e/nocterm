//! Supplied workspace observations are finite inputs to turn context projection.
use gpui_kit::{App, WeakEntity};
use nocterm_workspace::{ConnectionSummary, TerminalEntry, Workspace};
pub(in crate::thread) struct Resolution {
    terminals: Vec<TerminalEntry>,
    summaries: Vec<ConnectionSummary>,
}
impl Resolution {
    pub(in crate::thread) fn observe(workspace: &WeakEntity<Workspace>, cx: &App) -> Self {
        let Some(workspace) = workspace.upgrade() else {
            return Self {
                terminals: Vec::new(),
                summaries: Vec::new(),
            };
        };
        let workspace = workspace.read(cx);
        let summaries = workspace
            .connection_directory()
            .map(|directory| directory.connections(cx))
            .unwrap_or_default();
        Self {
            terminals: workspace.terminals(cx),
            summaries,
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
