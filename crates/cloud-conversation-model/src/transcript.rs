//! The neutral transcript of one Cloud chat. Plain display data: no row grants or admits
//! anything.
use crate::{CommandKey, ConversationDelta, IntentKey, RunKey};

/// Who a row is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowRole {
    /// An accepted prompt.
    User,
    Assistant,
    /// Model reasoning.
    Thought,
    /// A tool call and its outcome.
    Tool,
    /// A connection the runner asked for or opened.
    Session,
}

/// Where a tool call stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    AwaitingApproval {
        expires_at_ms: u64,
    },
    Approved,
    Denied,
    Succeeded,
    Failed,
    /// The effect may or may not have happened.
    Unknown,
    /// Continues as a background job.
    Background,
}

/// One tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRow {
    pub intent: IntentKey,
    pub tool: String,
    pub arguments_json: String,
    pub destination: Option<String>,
    pub mutates: bool,
    pub redacted: bool,
    pub state: ToolState,
    pub output_json: Option<String>,
    /// The outcome text the runner attached.
    pub outcome: Option<String>,
}

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    Denied,
    Expired,
    Cancelled,
    SourceChanged,
    Unavailable,
    Unsupported,
    Capacity,
    ConnectionLost,
}

/// Where a session stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// The runner waits for a device holding the key to approve the login.
    AwaitingConsent {
        expires_at_ms: u64,
    },
    Established,
    Lost(SessionEnd),
    Refused(SessionEnd),
}

/// One connection attempt of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    pub attempt: [u8; 16],
    pub principal: String,
    pub host_key_sha256: [u8; 32],
    /// An SSH login (true) or a local session on the runner.
    pub ssh: bool,
    pub terminal: Option<bool>,
    pub state: SessionState,
}

/// One row. `index` numbers rows from the first one the chat ever held, so a row keeps it when
/// earlier rows are dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: u64,
    pub role: RowRole,
    pub run: Option<RunKey>,
    /// Prompt or streamed text; empty for tool and session rows.
    pub text: String,
    pub images: u32,
    pub tool: Option<ToolRow>,
    pub session: Option<SessionRow>,
    pub at_ms: u64,
    pub updated_at_ms: u64,
}

/// Where a run stands, as the runner reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Queued,
    Running,
    WaitingApproval,
    NeedsGrant,
    NeedsUnlock,
    NeedsProviderAccess,
    OutcomeUnknown,
    Interrupted,
    Stopped,
    Completed,
}
impl RunStatus {
    /// Completed, stopped, interrupted or of unknown outcome.
    pub fn finished(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Stopped | Self::Interrupted | Self::OutcomeUnknown
        )
    }
}

/// Cumulative usage of a run; absent values are unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub thought_tokens: Option<u64>,
    pub cost_micros: Option<u64>,
}

/// The model a run uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelName {
    pub provider: String,
    pub model: String,
}

/// One run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub run: RunKey,
    pub status: RunStatus,
    pub usage: Usage,
    pub model: ModelName,
    /// The prompt command that started it.
    pub prompt_command: CommandKey,
}

/// A tool call waiting for the person's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub intent: IntentKey,
    pub run: RunKey,
    pub expires_at_ms: u64,
}

/// What a command this device sent asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Prompt,
    Stop,
    Allow,
    Answer,
    Provider,
    /// Binds or unbinds the chat's servers, or answers a certificate request.
    Servers,
}

/// Where a command this device sent stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandState {
    /// Signed and sealed; the hub has not confirmed it.
    Sending,
    /// The hub stored it; the runner has not answered.
    Delivered,
    /// Neither the hub nor the runner answered before the command expired.
    Unreachable,
    /// The hub or the runner refused it.
    Refused,
    /// The runner accepted it.
    Accepted,
}
impl CommandState {
    /// Nothing more will happen to the command.
    pub fn settled(self) -> bool {
        !matches!(self, Self::Sending | Self::Delivered)
    }
}

/// A command this device sent, shown until it settles. Device-local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCommand {
    pub command: CommandKey,
    pub kind: CommandKind,
    /// The prompt text of a prompt; empty otherwise.
    pub text: String,
    pub run: Option<RunKey>,
    pub state: CommandState,
    pub at_ms: i64,
}

/// The transcript a view holds, kept current by applying deltas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudTranscript {
    rows: Vec<Row>,
    runs: Vec<Run>,
    approvals: Vec<Approval>,
    pending: Vec<PendingCommand>,
    control_revision: u64,
    deleted: bool,
    omitted_earlier: bool,
}
impl CloudTranscript {
    pub fn new() -> Self {
        Self::default()
    }
    /// Applies `delta` by the rule [`ConversationDelta`] describes. Returns whether rows changed.
    pub fn apply(&mut self, delta: ConversationDelta) -> bool {
        let mut rows_changed = false;
        if delta.reset {
            *self = Self::default();
            rows_changed = true;
        }
        let before = self.rows.len();
        self.rows.retain(|row| row.index >= delta.first_index);
        rows_changed |= before != self.rows.len();
        if let Some(from) = delta.changed_from {
            self.rows.retain(|row| row.index < from);
            self.rows.extend(delta.rows);
            rows_changed = true;
        }
        if let Some(runs) = delta.runs {
            self.runs = runs;
        }
        if let Some(approvals) = delta.approvals {
            self.approvals = approvals;
        }
        if let Some(pending) = delta.pending {
            self.pending = pending;
        }
        self.control_revision = delta.control_revision;
        self.deleted = delta.deleted;
        self.omitted_earlier = delta.omitted_earlier;
        rows_changed
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }
    pub fn run(&self, run: RunKey) -> Option<&Run> {
        self.runs.iter().find(|known| known.run == run)
    }
    /// The run the chat shows: the latest one that has not finished, else the latest.
    pub fn shown_run(&self) -> Option<&Run> {
        self.runs
            .iter()
            .rev()
            .find(|run| !run.status.finished())
            .or_else(|| self.runs.last())
    }
    pub fn approvals(&self) -> &[Approval] {
        &self.approvals
    }
    pub fn approval(&self, intent: IntentKey) -> Option<&Approval> {
        self.approvals.iter().find(|known| known.intent == intent)
    }
    /// The commands this device sent, oldest first.
    pub fn pending(&self) -> &[PendingCommand] {
        &self.pending
    }
    pub fn pending_command(&self, command: CommandKey) -> Option<&PendingCommand> {
        self.pending.iter().find(|known| known.command == command)
    }
    /// The tool row of `intent`.
    pub fn tool(&self, intent: IntentKey) -> Option<&ToolRow> {
        self.rows
            .iter()
            .rev()
            .filter_map(|row| row.tool.as_ref())
            .find(|tool| tool.intent == intent)
    }
    pub fn control_revision(&self) -> u64 {
        self.control_revision
    }
    pub fn deleted(&self) -> bool {
        self.deleted
    }
    pub fn omitted_earlier(&self) -> bool {
        self.omitted_earlier
    }
    /// When the chat last changed, in milliseconds.
    pub fn updated_at_ms(&self) -> u64 {
        self.rows
            .iter()
            .map(|row| row.updated_at_ms)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests;
