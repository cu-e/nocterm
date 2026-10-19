//! Reconciles the ACP activity stream with the requests our bridge executed.
//! Neither provider titles nor today's attachment list establishes a destination.
#[cfg(test)]
use nocterm_ai::TerminalCall;
use nocterm_ai::model::ids::ToolId;
use nocterm_ai::model::tools::ToolRecord;
use nocterm_ai::{
    ToolFailure,
    thread::Entry,
    tool_display::{self, ToolDisplay},
};

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
    row: Option<ToolId>,
    ambiguous: bool,
}

impl Record {
    fn matches(&self, call: &ToolRecord) -> bool {
        self.row.as_ref() == Some(&call.id)
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
    pub(super) fn finish(
        &mut self,
        token: Option<(u64, u64)>,
        result: Result<serde_json::Value, ToolFailure>,
    ) {
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
    pub(super) fn finalize(&mut self, turn: u64) {
        if self.turn != Some(turn) {
            return;
        }
        for record in &mut self.records {
            if matches!(
                record.display.outcome,
                Some(tool_display::ToolOutcome::Pending)
            ) {
                record.display.finish(Err("Request cancelled.".into()));
            }
        }
    }

    pub(super) fn prepare(&mut self, turn: u64, entries: usize) {
        if self.turn != Some(turn) {
            self.turn = Some(turn);
            self.start = entries;
            self.records.clear();
        }
    }

    pub(super) fn normalize(
        &mut self,
        entries: &[Entry],
    ) -> Vec<super::document::ToolMetadataPatch> {
        self.revoke_ambiguous_destinations(entries);
        let mut changed = Vec::new();
        for index in self.start..entries.len() {
            let Entry::Tool(call) = &entries[index] else {
                continue;
            };
            let bound = self
                .records
                .iter()
                .position(|record| record.row.as_ref() == Some(&call.id));
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
            let Entry::Tool(call) = &entries[index] else {
                continue;
            };
            record.row = Some(call.id.clone());
            if let Ok(value) = serde_json::to_value(&record.display)
                && call
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get(tool_display::META_KEY))
                    != Some(&value)
            {
                changed.push(super::document::ToolMetadataPatch::new(index, value));
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

    pub(super) fn record_raw(
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
}

#[cfg(test)]
mod tests;
