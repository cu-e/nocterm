//! The neutral Cloud transcript as the thread the conversation renderer draws: one entry per
//! row, rebuilt only from the first changed row on.
use nocterm_ai::{
    conversation::ConversationPhase,
    model::{
        content::Content,
        tools::{ToolRecord, ToolStatus},
        usage::TokenTotals,
    },
    thread::{Entry, ThreadState},
    tool_display::{ToolDisplay, ToolOutcome},
};
use nocterm_cloud_conversation_model::{
    CloudTranscript, ConversationDelta, Row, RowRole, RunStatus, SessionEnd, SessionRow,
    SessionState, ToolOutcome as CloudOutcome, ToolRow, ToolState,
};
use serde_json::Value;

/// Expensive text/JSON conversion of the incoming rows, prepared off the foreground.
pub(crate) struct PreparedRows {
    entries: Vec<Entry>,
    indices: Vec<u64>,
    times: Vec<u64>,
}
impl PreparedRows {
    pub(crate) fn new(delta: &ConversationDelta) -> Self {
        let rows = if delta.changed_from.is_some() {
            delta.rows.as_slice()
        } else {
            &[]
        };
        Self {
            entries: rows.iter().map(entry).collect(),
            indices: rows.iter().map(|row| row.index).collect(),
            times: rows.iter().map(|row| row.at_ms / 1000).collect(),
        }
    }
}

/// The thread of one transcript, and which row each entry shows.
#[derive(Default)]
pub(crate) struct ThreadMirror {
    thread: ThreadState,
    indices: Vec<u64>,
    ordered: bool,
}
impl ThreadMirror {
    pub(crate) fn thread(&self) -> &ThreadState {
        &self.thread
    }
    /// Existing rows follow the same retention rule as the neutral transcript. A normal
    /// ordered append/replacement cuts by binary search; unusual unordered input retains
    /// the original public delta semantics without a sorting assumption.
    pub(crate) fn accept(&mut self, delta: &ConversationDelta, rows: PreparedRows) {
        self.retain(delta);
        self.ordered = (self.ordered || self.indices.is_empty())
            && rows.indices.windows(2).all(|pair| pair[0] <= pair[1])
            && self
                .indices
                .last()
                .zip(rows.indices.first())
                .is_none_or(|(last, first)| last <= first);
        self.thread.entries.extend(rows.entries);
        self.thread.times.extend(rows.times);
        self.indices.extend(rows.indices);
    }
    fn retain(&mut self, delta: &ConversationDelta) {
        if delta.reset {
            self.clear();
        } else if self.ordered {
            let first = self
                .indices
                .partition_point(|index| *index < delta.first_index);
            let end = delta.changed_from.map_or(self.indices.len(), |from| {
                self.indices.partition_point(|index| *index < from)
            });
            self.thread.truncate(end);
            self.indices.truncate(end);
            let first = first.min(end);
            if first != 0 {
                self.thread.entries.drain(..first);
                self.thread.times.drain(..first);
                self.indices.drain(..first);
            }
        } else {
            retain_entries(&mut self.thread.entries, &self.indices, delta);
            retain_entries(&mut self.thread.times, &self.indices, delta);
            self.indices.retain(|index| keeps(*index, delta));
        }
    }
    pub(crate) fn sync_usage(&mut self, transcript: &CloudTranscript) {
        self.thread.tokens = transcript.shown_run().and_then(|run| {
            let usage = run.usage;
            let mut totals = TokenTotals::new(
                usage.total_tokens?,
                usage.input_tokens?,
                usage.output_tokens?,
            );
            totals.thought = usage.thought_tokens;
            totals.cache_read = usage.cached_input_tokens;
            Some(totals)
        });
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}
fn keeps(index: u64, delta: &ConversationDelta) -> bool {
    index >= delta.first_index && delta.changed_from.is_none_or(|from| index < from)
}
fn retain_entries<T>(values: &mut Vec<T>, indices: &[u64], delta: &ConversationDelta) {
    let mut position = 0;
    values.retain(|_| {
        let kept = keeps(indices[position], delta);
        position += 1;
        kept
    });
}

fn entry(row: &Row) -> Entry {
    match row.role {
        RowRole::User => {
            let mut text = row.text.clone();
            if row.images > 0 {
                text.push_str(&format!("\n[{} image(s)]", row.images));
            }
            Entry::User(vec![Content::text(text)])
        }
        RowRole::Assistant => Entry::Agent(row.text.clone()),
        RowRole::Thought => Entry::Thought(row.text.clone()),
        RowRole::Tool => match &row.tool {
            Some(tool) => Entry::Tool(tool_record(row.index, tool)),
            None => Entry::Agent(String::new()),
        },
        RowRole::Session => match &row.session {
            Some(session) => Entry::Tool(session_record(row.index, session)),
            None => Entry::Agent(String::new()),
        },
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

fn tool_record(index: u64, tool: &ToolRow) -> ToolRecord {
    let status = match tool.state {
        ToolState::Succeeded => ToolStatus::Succeeded,
        ToolState::Failed | ToolState::Denied => ToolStatus::Failed,
        ToolState::Unknown => ToolStatus::Pending,
        ToolState::Running
        | ToolState::AwaitingApproval { .. }
        | ToolState::Approved
        | ToolState::Background => ToolStatus::Running,
    };
    let outcome = match (&tool.outcome, tool.state) {
        (Some(CloudOutcome::Ok(text)), _) => ToolOutcome::Ok(json(text)),
        (Some(CloudOutcome::Err(text)), _) => ToolOutcome::Err(text.clone()),
        (Some(CloudOutcome::Pending) | None, ToolState::Denied) => {
            ToolOutcome::Err("Denied".to_owned())
        }
        (Some(CloudOutcome::Pending) | None, _) => ToolOutcome::Pending,
    };
    let display = ToolDisplay {
        version: 2,
        tool: tool.tool.clone(),
        arguments: json(&tool.arguments_json),
        destination: tool.destination.clone(),
        redacted: tool.redacted,
        outcome: Some(outcome),
    };
    // The row's title is the core's, never rebuilt from the tool name here.
    let mut record = ToolRecord::new(format!("{index}-{}", tool.intent), tool.title.clone())
        .name(tool.tool.clone())
        .status(status);
    record.raw_output = tool.output_json.as_deref().map(json);
    display.attach(&mut record);
    record
}

fn session_end(end: SessionEnd) -> &'static str {
    match end {
        SessionEnd::Denied => "denied",
        SessionEnd::Expired => "expired",
        SessionEnd::Cancelled => "cancelled",
        SessionEnd::SourceChanged => "its source changed",
        SessionEnd::Unavailable => "unavailable",
        SessionEnd::Unsupported => "unsupported",
        SessionEnd::Capacity => "at capacity",
        SessionEnd::ConnectionLost => "connection lost",
    }
}

/// A connection the runner asked for or opened, shown as a tool row.
fn session_record(index: u64, session: &SessionRow) -> ToolRecord {
    let (state, status) = match session.state {
        SessionState::AwaitingConsent { .. } => (
            "waiting for a device to approve".to_owned(),
            ToolStatus::Pending,
        ),
        SessionState::Established => ("connected".to_owned(), ToolStatus::Succeeded),
        SessionState::Lost(end) => (format!("lost: {}", session_end(end)), ToolStatus::Failed),
        SessionState::Refused(end) => {
            (format!("refused: {}", session_end(end)), ToolStatus::Failed)
        }
    };
    let kind = if session.ssh { "SSH" } else { "Session" };
    let title = format!("{kind} · {} · {state}", session.principal);
    let id: String = session
        .attempt
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ToolRecord::new(format!("{index}-session-{id}"), title)
        .name("session")
        .status(status)
}

/// What the chat shows as its phase: the shown run's, or deleted.
pub(crate) fn phase(transcript: &CloudTranscript) -> ConversationPhase {
    if transcript.deleted() {
        return ConversationPhase::Deleted;
    }
    match transcript.shown_run().map(|run| run.status) {
        None => ConversationPhase::Idle,
        Some(RunStatus::Queued) => ConversationPhase::Queued,
        Some(RunStatus::Running) => ConversationPhase::Running,
        Some(RunStatus::WaitingApproval) => ConversationPhase::WaitingApproval,
        Some(RunStatus::NeedsGrant) => ConversationPhase::NeedsGrant,
        Some(RunStatus::NeedsUnlock) => ConversationPhase::NeedsUnlock,
        Some(RunStatus::NeedsProviderAccess) => ConversationPhase::NeedsProviderAccess,
        Some(RunStatus::OutcomeUnknown) => ConversationPhase::OutcomeUnknown,
        Some(RunStatus::Interrupted) => ConversationPhase::Interrupted,
        Some(RunStatus::Stopped) => ConversationPhase::Stopped,
        Some(RunStatus::Completed) => ConversationPhase::Completed,
    }
}
