//! Reconciles the provider activity stream with the requests our bridge executed.
//! Neither provider titles nor today's attachment list establishes a destination.
#[cfg(test)]
use nocterm_ai::TerminalCall;
use nocterm_ai::{
    ToolFailure,
    history::physical::{RowBody, RowSlot, ToolSlot},
    model::{controls::Presence, ids::ToolId},
    tool_display::{self, ToolDisplay},
};

/// A client display which must be copied into one retained tool row.
pub(super) struct DisplayPatch {
    row: usize,
    display: ToolDisplay,
}
impl DisplayPatch {
    pub(super) fn row(&self) -> usize {
        self.row
    }
    pub(super) fn display(&self) -> &ToolDisplay {
        &self.display
    }
}
fn tool(row: &RowSlot) -> Option<&ToolSlot> {
    match row.view() {
        RowBody::Tool(slot) => Some(slot),
        _ => None,
    }
}

/// The row of the tool call `id`, when the document retains it.
pub(super) fn tool_row(rows: &[RowSlot], id: &str) -> Option<usize> {
    rows.iter()
        .position(|row| tool(row).is_some_and(|call| call.id().as_str() == id))
}

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
    fn matches(&self, call: &ToolSlot) -> bool {
        self.row.as_ref().is_some_and(|row| *row == *call.id())
            || tool_display::raw_envelope_managed(call, &self.server)
                .is_some_and(|request| request == self.request)
    }

    fn revoke_destination(&mut self) {
        self.ambiguous = true;
        self.display.destination = Presence::Null;
        self.display.outcome = Presence::Absent;
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
            && let Err(error) = record.display.finish(result)
        {
            tracing::warn!(%error, "tool display result could not be retained");
            record.display.outcome =
                Presence::Value(tool_display::ToolOutcome::Failed(error.to_string()));
        }
    }
    pub(super) fn finalize(&mut self, turn: u64) {
        if self.turn != Some(turn) {
            return;
        }
        for record in &mut self.records {
            if matches!(
                record.display.outcome,
                Presence::Value(tool_display::ToolOutcome::Pending)
            ) {
                // Cancellation has no JSON conversion or retained payload allocation.
                record.display.outcome = Presence::Value(tool_display::ToolOutcome::Failed(
                    "Request cancelled.".into(),
                ));
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

    pub(super) fn normalize(&mut self, rows: &[RowSlot]) -> Vec<DisplayPatch> {
        self.revoke_ambiguous_destinations(rows);
        let mut changed = Vec::new();
        for index in self.start..rows.len() {
            let Some(call) = tool(&rows[index]) else {
                continue;
            };
            let id = call.id();
            let bound = self
                .records
                .iter()
                .position(|record| record.row.as_ref() == Some(&*id));
            let candidate = bound.or_else(|| {
                let matches: Vec<_> = self
                    .records
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| {
                        !record.ambiguous
                            && record.row.is_none()
                            && tool_display::raw_envelope_managed(call, &record.server)
                                .is_some_and(|request| request == record.request)
                    })
                    .map(|(index, _)| index)
                    .collect();
                if matches.len() != 1 {
                    return None;
                }
                let record = &self.records[matches[0]];
                let candidates = rows[self.start..]
                    .iter()
                    .filter_map(tool)
                    .filter(|other| {
                        ToolDisplay::from_managed(other).is_none()
                            && tool_display::raw_envelope_managed(other, &record.server)
                                .is_some_and(|request| request == record.request)
                    })
                    .count();
                (candidates == 1).then_some(matches[0])
            });
            let Some(candidate) = candidate else {
                continue;
            };
            let record = &mut self.records[candidate];
            record.row = Some((*id).clone());
            if call
                .client_display()
                .is_none_or(|display| **display != record.display)
            {
                changed.push(DisplayPatch {
                    row: index,
                    display: record.display.clone(),
                });
            }
        }
        changed
    }

    fn revoke_ambiguous_destinations(&mut self, rows: &[RowSlot]) {
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
                    || rows[self.start..]
                        .iter()
                        .filter_map(tool)
                        .filter(|call| record.matches(call))
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
