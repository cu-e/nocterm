//! Reconciles the ACP activity stream with the requests our bridge executed.
//! Neither provider titles nor today's attachment list establishes a destination.
use gpui_kit::{App, Context};
use nocterm_ai::{
    BridgeCall, BridgeRejection, TerminalCall, acp,
    thread::{Entry, ThreadChange},
    tool_display::{self, ToolDisplay},
};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use nocterm_workspace::TerminalEntry;

use super::AgentThread;

#[derive(Default)]
pub(super) struct ToolDisplays {
    turn: Option<u64>,
    start: usize,
    records: Vec<Record>,
    next_id: u64,
}

struct Record {
    server: String,
    id: u64,
    request: (String, serde_json::Value),
    display: ToolDisplay,
    row: Option<acp::ToolCallId>,
    ambiguous: bool,
}

impl Record {
    fn matches(&self, call: &acp::ToolCall) -> bool {
        self.row.as_ref() == Some(&call.tool_call_id)
            || tool_display::raw_envelope(call, &self.server)
                .is_some_and(|request| request == self.request)
    }

    fn revoke_destination(&mut self) {
        self.ambiguous = true;
        self.display.destination = None;
        self.display.outcome = None;
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
                            && tool_display::raw_envelope(call, &record.server)
                                .is_some_and(|request| request == record.request)
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
                            && tool_display::raw_envelope(other, &record.server)
                                .is_some_and(|request| request == record.request)
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

    fn record_raw(
        &mut self,
        server: String,
        request: (String, serde_json::Value),
        display: ToolDisplay,
    ) -> Option<u64> {
        if self.records.len() >= 128 {
            for record in &mut self.records {
                if record.server == server && record.request == request {
                    record.revoke_destination();
                }
            }
            return None;
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.records.push(Record {
            id,
            server,
            request,
            display,
            row: None,
            ambiguous: false,
        });
        Some(id)
    }

    #[cfg(test)]
    fn record(&mut self, server: String, request: &TerminalCall, display: ToolDisplay) {
        let (tool, arguments) = tool_display::request(request);
        self.record_raw(server, (tool.into(), arguments), display);
    }

    fn finish(&mut self, token: Option<(u64, u64)>, result: Result<serde_json::Value, String>) {
        let Some((turn, id)) = token else {
            return;
        };
        if self.turn != Some(turn) {
            return;
        }
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.id == id && !record.ambiguous)
        {
            record.display.finish(result);
        }
    }
}

impl AgentThread {
    pub(crate) fn apply_presented_update(
        &mut self,
        mut update: acp::SessionUpdate,
    ) -> ThreadChange {
        self.tool_displays
            .prepare(self.lifecycle.turn(), self.state.entries.len());
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
        call: &mut BridgeCall,
        provided_entry: Option<&TerminalEntry>,
        cx: &mut Context<Self>,
    ) {
        let Some(registration) = &self.registration() else {
            return;
        };
        let server = tool_display::bridge_server_name(registration.id);
        let (tool, normalized) = tool_display::request(&call.call);
        let arguments = call.arguments.clone().unwrap_or(normalized);
        let entry = self
            .resolved(cx)
            .into_iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
            .map(|(_, entry, _)| entry);
        let destination = self.tool_destination(&call.call, provided_entry.or(entry.as_ref()), cx);
        let mut display = ToolDisplay::requested(tool.into(), arguments.clone());
        display.destination = destination.map(|text| nocterm_ai::redact::redact(&text));
        if cx
            .setting::<nocterm_ai::AiSettings>()
            .approval
            .redact_secrets
        {
            display.redact();
        }
        self.tool_displays
            .prepare(self.lifecycle.turn(), self.state.entries.len());
        call.display_token = self
            .tool_displays
            .record_raw(server, (tool.into(), arguments), display)
            .map(|id| (self.lifecycle.turn(), id));
        self.normalize_tool_displays();
        cx.notify();
    }

    pub(crate) fn record_rejected_tool(
        &mut self,
        rejected: BridgeRejection,
        cx: &mut Context<Self>,
    ) {
        if !cx.ai_enabled()
            || !self.lifecycle.accepts_updates()
            || self.lifecycle.stopped()
            || self.registration().as_ref().map(|value| value.id) != Some(rejected.registration_id)
        {
            return;
        }
        let server = tool_display::bridge_server_name(rejected.registration_id);
        let mut display = ToolDisplay::requested(rejected.tool.clone(), rejected.arguments.clone());
        display.finish(Err(rejected.error));
        if cx
            .setting::<nocterm_ai::AiSettings>()
            .approval
            .redact_secrets
        {
            display.redact();
        }
        self.tool_displays
            .prepare(self.lifecycle.turn(), self.state.entries.len());
        self.tool_displays
            .record_raw(server, (rejected.tool, rejected.arguments), display);
        self.normalize_tool_displays();
        cx.notify();
    }

    /// Every response while the chat exists passes through its recorded result.
    pub(super) fn finish(&mut self, call: BridgeCall, result: Result<serde_json::Value, String>) {
        self.tool_displays
            .finish(call.display_token, result.clone());
        self.normalize_tool_displays();
        let _ = call.respond.send(result);
    }

    pub(super) fn finalize_tool_displays(&mut self) {
        if self.tool_displays.turn != Some(self.lifecycle.turn()) {
            return;
        }
        for record in &mut self.tool_displays.records {
            if matches!(
                record.display.outcome,
                Some(tool_display::ToolOutcome::Pending)
            ) {
                record.display.finish(Err("Request cancelled.".into()));
            }
        }
        self.normalize_tool_displays();
    }

    fn normalize_tool_displays(&mut self) {
        let changed = self.tool_displays.normalize(&mut self.state.entries);
        for index in changed {
            self.mark_dirty(index);
        }
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
