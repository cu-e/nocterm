//! Reconciles the ACP activity stream with the requests our bridge executed.
//! Neither provider titles nor today's attachment list establishes a destination.
use gpui_kit::{App, Context};
use nocterm_ai::{
    TerminalCall, acp,
    thread::{Entry, ThreadChange},
    tool_display::{self, ToolDisplay},
};
use nocterm_ui::SettingsExt as _;
use nocterm_workspace::TerminalEntry;

use super::{AgentThread, bridge_server_name};

#[derive(Default)]
pub(super) struct ToolDisplays {
    turn: Option<u64>,
    start: usize,
    records: Vec<Record>,
}

struct Record {
    server: String,
    request: (&'static str, serde_json::Value),
    display: ToolDisplay,
    row: Option<acp::ToolCallId>,
    ambiguous: bool,
}

impl Record {
    fn matches(&self, call: &acp::ToolCall) -> bool {
        self.row.as_ref() == Some(&call.tool_call_id)
            || tool_display::envelope(call, &self.server)
                .is_some_and(|request| tool_display::request(&request) == self.request)
    }

    fn revoke_destination(&mut self) {
        self.ambiguous = true;
        self.display.destination = None;
    }
}

impl ToolDisplays {
    fn prepare(&mut self, turn: u64, entries: usize) {
        if self.turn != Some(turn) {
            self.turn = Some(turn);
            self.start = entries;
            self.records.clear();
        }
    }

    fn normalize(&mut self, entries: &mut [Entry]) -> Vec<usize> {
        self.revoke_ambiguous_destinations(entries);
        let mut changed = Vec::new();
        for index in self.start..entries.len() {
            let Entry::Tool(call) = &entries[index] else {
                continue;
            };
            let bound = self
                .records
                .iter()
                .position(|record| record.row.as_ref() == Some(&call.tool_call_id));
            let candidate = bound.or_else(|| {
                let matches: Vec<_> = self
                    .records
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| {
                        !record.ambiguous
                            && record.row.is_none()
                            && tool_display::envelope(call, &record.server).is_some_and(|request| {
                                tool_display::request(&request) == record.request
                            })
                    })
                    .map(|(index, _)| index)
                    .collect();
                if matches.len() != 1 {
                    return None;
                }
                let record = &self.records[matches[0]];
                let rows = entries[self.start..]
                    .iter()
                    .filter(|entry| {
                        let Entry::Tool(other) = entry else {
                            return false;
                        };
                        ToolDisplay::from_call(other).is_none()
                            && tool_display::envelope(other, &record.server).is_some_and(
                                |request| tool_display::request(&request) == record.request,
                            )
                    })
                    .count();
                (rows == 1).then_some(matches[0])
            });
            let Some(candidate) = candidate else {
                continue;
            };
            let record = &mut self.records[candidate];
            let Entry::Tool(call) = &mut entries[index] else {
                continue;
            };
            record.row = Some(call.tool_call_id.clone());
            let old = call.meta.clone();
            record.display.attach(call);
            if old != call.meta {
                changed.push(index);
            }
        }
        changed
    }

    fn revoke_ambiguous_destinations(&mut self, entries: &[Entry]) {
        let ambiguous: Vec<_> = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record.ambiguous
                    || self
                        .records
                        .iter()
                        .filter(|other| {
                            other.server == record.server && other.request == record.request
                        })
                        .count()
                        > 1
                    || entries[self.start..]
                        .iter()
                        .filter(|entry| matches!(entry, Entry::Tool(call) if record.matches(call)))
                        .count()
                        > 1
            })
            .map(|(index, _)| index)
            .collect();
        for index in ambiguous {
            self.records[index].revoke_destination();
        }
    }

    fn record(&mut self, server: String, request: &TerminalCall, display: ToolDisplay) {
        let request = tool_display::request(request);
        if self.records.len() >= 128 {
            // A request beyond the tracking budget can still make an earlier
            // association ambiguous. Revoke that host without losing its frozen
            // payload, and keep the ambiguity marker until the next turn.
            for record in &mut self.records {
                if record.server == server && record.request == request {
                    record.revoke_destination();
                }
            }
            return;
        }
        self.records.push(Record {
            server,
            request,
            display,
            row: None,
            ambiguous: false,
        });
    }
}

impl AgentThread {
    pub(crate) fn apply_presented_update(
        &mut self,
        mut update: acp::SessionUpdate,
    ) -> ThreadChange {
        self.tool_displays
            .prepare(self.turn, self.state.entries.len());
        tool_display::strip_provider_meta(&mut update);
        let change = self.state.apply(update);
        let changed = self.tool_displays.normalize(&mut self.state.entries);
        for index in changed {
            self.mark_dirty(index);
        }
        change
    }

    pub(crate) fn record_tool_display(
        &mut self,
        request: &TerminalCall,
        entry: Option<&TerminalEntry>,
        cx: &mut Context<Self>,
    ) {
        let Some(registration) = &self.registration() else {
            return;
        };
        let server = bridge_server_name(registration.id);
        let destination = self.tool_destination(request, entry, cx);
        let mut display = ToolDisplay::new(request, destination);
        if cx
            .setting::<nocterm_settings::AiSettings>()
            .approval
            .redact_secrets
        {
            display.redact();
        }
        display.destination = display
            .destination
            .map(|text| nocterm_ai::redact::redact(&text));
        self.tool_displays
            .prepare(self.turn, self.state.entries.len());
        // Activity metadata is a bounded per-turn aid. Unmatched records never
        // authorize a guessed association when providers omit identifiers.
        self.tool_displays.record(server, request, display);
        let changed = self.tool_displays.normalize(&mut self.state.entries);
        for index in changed {
            self.mark_dirty(index);
        }
        cx.notify();
    }

    fn tool_destination(
        &mut self,
        request: &TerminalCall,
        entry: Option<&TerminalEntry>,
        cx: &App,
    ) -> Option<String> {
        if let Some(entry) = entry {
            let info = entry.access.info(cx)?;
            if info.local {
                return Some("This computer".into());
            }
            let target = info.target?;
            return Some(format!("{} · {target}", entry.title));
        }
        let server = request.server_id()?;
        self.offline_servers(cx)
            .into_iter()
            .find(|(descriptor, _)| descriptor.server_id == server)
            .map(|(descriptor, summary)| format!("{} · {}", descriptor.name, summary.target))
    }
}

#[cfg(test)]
mod tests;
