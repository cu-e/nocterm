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
    CloudTranscript, Row, RowRole, RunStatus, SessionEnd, SessionRow, SessionState,
    ToolOutcome as CloudOutcome, ToolRow, ToolState,
};
use serde_json::Value;

/// The thread of one transcript, and which row each entry shows.
#[derive(Default)]
pub(crate) struct ThreadMirror {
    thread: ThreadState,
    indices: Vec<u64>,
}
impl ThreadMirror {
    pub(crate) fn thread(&self) -> &ThreadState {
        &self.thread
    }
    /// Brings the thread up to `transcript`: entries of rows that did not change stay.
    pub(crate) fn follow(&mut self, transcript: &CloudTranscript, changed_from: Option<u64>) {
        let rows = transcript.rows();
        let kept = match changed_from {
            Some(from) => rows.iter().take_while(|row| row.index < from).count(),
            None => rows.len(),
        };
        let unchanged = self.indices.len() >= kept
            && self.indices[..kept]
                .iter()
                .zip(rows)
                .all(|(index, row)| *index == row.index);
        let keep = if unchanged { kept } else { 0 };
        self.thread.truncate(keep);
        self.indices.truncate(keep);
        for row in &rows[keep..] {
            self.thread.entries.push(entry(row));
            self.thread.times.push(row.at_ms / 1000);
            self.indices.push(row.index);
        }
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

/// Roughly what the transcript keeps in memory, for its admission.
pub(crate) fn resident_bytes(transcript: &CloudTranscript) -> usize {
    let rows: usize = transcript
        .rows()
        .iter()
        .map(|row| {
            256 + row.text.len()
                + row.tool.as_ref().map_or(0, |tool| {
                    tool.arguments_json.len()
                        + tool.output_json.as_ref().map_or(0, String::len)
                        + tool
                            .outcome
                            .as_ref()
                            .map_or(0, |outcome| outcome.text().len())
                        + 1024
                })
        })
        .sum();
    // The thread holds a second copy of the text.
    rows.saturating_mul(2)
        .saturating_add(transcript.runs().len() * 256)
        .saturating_add(4096)
}
