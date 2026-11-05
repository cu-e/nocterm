//! Incremental admission costs of the accepted neutral transcript and its renderer mirror.
//! Entries are derived only from accepted deltas; a refused candidate changes nothing.
use nocterm_cloud_conversation_model::{ConversationDelta, Row, ToolOutcome};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default)]
struct Retained {
    rows: usize,
    runs: usize,
    approvals: usize,
    pending: usize,
    servers: usize,
}
impl Retained {
    fn bytes(self) -> usize {
        // Neutral rows and the mirror each retain text; Residency separately keeps its
        // existing growth slack. This is conservative admission, not measured process RSS.
        self.rows
            .saturating_mul(2)
            .saturating_add(self.runs)
            .saturating_add(self.approvals)
            .saturating_add(self.pending)
            .saturating_add(self.servers)
            .saturating_add(4096)
    }
}

#[derive(Default)]
pub(crate) struct TranscriptCost {
    rows: BTreeMap<u64, usize>,
    retained: Retained,
}

pub(crate) struct CandidateCost(Retained);
impl CandidateCost {
    pub(crate) fn bytes(&self) -> usize {
        self.0.bytes()
    }
}
impl TranscriptCost {
    pub(crate) fn bytes(&self) -> usize {
        self.retained.bytes()
    }
    /// Reads only removed and incoming rows, and metadata that the delta replaces.
    /// The map is accounting, never a second transcript or source of authorization.
    pub(crate) fn prepare(&self, delta: &ConversationDelta) -> CandidateCost {
        let mut next = if delta.reset {
            Retained::default()
        } else {
            self.retained
        };
        if !delta.reset {
            next.rows = next.rows.saturating_sub(self.removed(delta));
        }
        if delta.changed_from.is_some() {
            for row in &delta.rows {
                next.rows = next.rows.saturating_add(row_bytes(row));
            }
        }
        if let Some(runs) = &delta.runs {
            next.runs = runs
                .iter()
                .fold(runs.capacity().saturating_mul(256), |cost, run| {
                    cost.saturating_add(run.model.provider.capacity())
                        .saturating_add(run.model.model.capacity())
                });
        }
        if let Some(approvals) = &delta.approvals {
            next.approvals = approvals.capacity().saturating_mul(128);
        }
        if let Some(pending) = &delta.pending {
            next.pending = pending
                .iter()
                .fold(pending.capacity().saturating_mul(256), |cost, command| {
                    cost.saturating_add(command.text.capacity())
                });
        }
        if let Some(servers) = &delta.servers {
            next.servers = servers.servers.iter().fold(
                servers.servers.capacity().saturating_mul(256),
                |cost, server| {
                    cost.saturating_add(server.host.capacity())
                        .saturating_add(server.user.capacity())
                },
            );
        }
        CandidateCost(next)
    }
    fn removed(&self, delta: &ConversationDelta) -> usize {
        if delta
            .changed_from
            .is_some_and(|from| from <= delta.first_index)
        {
            return self.retained.rows;
        }
        let mut removed = self
            .rows
            .range(..delta.first_index)
            .fold(0usize, |cost, (_, bytes)| cost.saturating_add(*bytes));
        if let Some(from) = delta.changed_from {
            removed = self
                .rows
                .range(from..)
                .fold(removed, |cost, (_, bytes)| cost.saturating_add(*bytes));
        }
        removed
    }
    /// Called only after admission, immediately before applying this same owned delta.
    pub(crate) fn accept(&mut self, delta: &ConversationDelta, candidate: CandidateCost) {
        if delta.reset {
            self.rows.clear();
        } else {
            if self
                .rows
                .first_key_value()
                .is_some_and(|(first, _)| *first < delta.first_index)
            {
                self.rows = self.rows.split_off(&delta.first_index);
            }
            if let Some(from) = delta.changed_from {
                self.rows.split_off(&from);
            }
        }
        if delta.changed_from.is_some() {
            for row in &delta.rows {
                let cost = self.rows.entry(row.index).or_default();
                *cost = cost.saturating_add(row_bytes(row));
            }
        }
        self.retained = candidate.0;
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

fn row_bytes(row: &Row) -> usize {
    // Includes the row/mirror slots and the per-index map node within the existing
    // two-copy estimate plus Residency's separate conservative growth slack.
    let mut cost = 256usize.saturating_add(row.text.capacity());
    if let Some(tool) = &row.tool {
        cost = cost
            .saturating_add(1024)
            .saturating_add(tool.title.capacity())
            .saturating_add(tool.tool.capacity())
            .saturating_add(tool.arguments_json.capacity())
            .saturating_add(tool.destination.as_ref().map_or(0, String::capacity))
            .saturating_add(tool.output_json.as_ref().map_or(0, String::capacity))
            .saturating_add(tool.outcome.as_ref().map_or(0, |outcome| match outcome {
                ToolOutcome::Pending => 0,
                ToolOutcome::Ok(text) | ToolOutcome::Err(text) => text.capacity(),
            }));
    }
    if let Some(session) = &row.session {
        cost = cost
            .saturating_add(session.principal.capacity())
            .saturating_add(256);
    }
    cost
}
