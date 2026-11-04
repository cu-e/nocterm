//! Chat summaries a runner derives from its own journal and publishes to its directory
//! stream (ADR 0135).
//!
//! [`ChatSummaryFacts`] reduces the observations the runner journals for one chat: the
//! first accepted prompt, the latest model, run statuses, open approvals and deletion.
//! [`ChatSummaryTracker`] adds the revision ledger and the publication cadence. The values
//! are neutral: the hub-io adapter behind [`ChatSummaryDirectory`] reads the current head,
//! formats the title with the contract, signs and seals each revision. The runner journal
//! keeps the newest staged revision behind [`ChatSummaryJournal`], so a retry publishes the
//! same sealed bytes and a revision only grows.
mod tracker;
pub use tracker::{ChatSummaryTracker, SummaryStep};

use super::{JournalError, JournalFuture};
use crate::{
    authority::{RunBinding, StreamBinding},
    coordinator::RunStatus,
    engine::EnginePrompt,
    identity::{ChatKeyEpoch, IntentId, Timestamp},
    inference::ModelSelection,
    payload::OpaqueBytes,
};
use nocterm_ai::model::content::Content;
use std::collections::BTreeMap;

/// The first prompt line a summary keeps. The contract cuts the title from it at 120 bytes;
/// replacing control characters never shrinks a line below half its bytes, so 256 bytes
/// always cover the title.
pub const MAX_SUMMARY_PROMPT_LINE_BYTES: usize = 256;
/// The largest sealed summary the runner journal stores.
pub const MAX_SEALED_CHAT_SUMMARY_BYTES: usize = 4096;
/// At most one summary per chat in this interval; a deletion is published at once.
pub const SUMMARY_INTERVAL_MS: u64 = 5_000;
/// The longest wait between attempts after failed publications.
pub const SUMMARY_RETRY_CEILING_MS: u64 = 5 * 60_000;
const MAX_TRACKED_RUNS: usize = 64;
const MAX_TRACKED_APPROVALS: usize = 64;

/// What a chat is doing, as its runner last journaled it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatActivity {
    Idle,
    Running,
    WaitingApproval,
    NeedsGrant,
    NeedsProvider,
    NeedsSsh,
    Failed,
    Deleted,
}

/// One journaled fact about a chat, as the summary needs it.
#[derive(Clone, Copy, Debug)]
pub enum SummaryObservation<'a> {
    /// A prompt was accepted and allocated `run`.
    Prompt {
        run: &'a RunBinding,
        prompt: &'a EnginePrompt,
        model: &'a ModelSelection,
    },
    Status {
        run: &'a RunBinding,
        status: RunStatus,
    },
    Approval {
        run: &'a RunBinding,
        intent: IntentId,
        open: bool,
    },
    Deleted,
    /// Any other record: only the last activity moves.
    Activity,
}

/// The reduced facts of one chat. Bounded: a chat tracks at most 64 unfinished runs and 64
/// open approvals; further ones only move the last activity.
#[derive(Clone, Debug, Default)]
pub struct ChatSummaryFacts {
    created_at: Option<Timestamp>,
    last_activity: Option<Timestamp>,
    prompt_line: Option<String>,
    model: Option<ModelSelection>,
    runs: BTreeMap<RunBinding, RunStatus>,
    approvals: BTreeMap<IntentId, RunBinding>,
    failed: bool,
    deleted: bool,
}
impl ChatSummaryFacts {
    /// The facts of a chat whose records are no longer readable, after its deletion.
    pub fn deleted_at(at: Timestamp) -> Self {
        let mut facts = Self::default();
        facts.observe(SummaryObservation::Deleted, at);
        facts
    }
    pub fn observe(&mut self, observation: SummaryObservation<'_>, at: Timestamp) {
        self.created_at = Some(self.created_at.map_or(at, |created| created.min(at)));
        self.last_activity = Some(self.last_activity.map_or(at, |last| last.max(at)));
        if self.deleted {
            return;
        }
        match observation {
            SummaryObservation::Prompt { run, prompt, model } => {
                if self.prompt_line.is_none() {
                    self.prompt_line = prompt_line(prompt);
                }
                self.model = Some(model.clone());
                self.track(run, RunStatus::Queued);
            }
            SummaryObservation::Status { run, status } => self.status(run, status),
            SummaryObservation::Approval { run, intent, open } => {
                if !open {
                    self.approvals.remove(&intent);
                } else if self.approvals.len() < MAX_TRACKED_APPROVALS {
                    self.approvals.insert(intent, run.clone());
                }
            }
            SummaryObservation::Deleted => {
                self.deleted = true;
                self.runs.clear();
                self.approvals.clear();
            }
            SummaryObservation::Activity => {}
        }
    }
    fn status(&mut self, run: &RunBinding, status: RunStatus) {
        let failed = match status {
            RunStatus::Completed | RunStatus::Stopped => false,
            RunStatus::OutcomeUnknown | RunStatus::Interrupted => true,
            _ => return self.track(run, status),
        };
        self.runs.remove(run);
        self.approvals.retain(|_, owner| owner != run);
        self.failed = failed;
    }
    fn track(&mut self, run: &RunBinding, status: RunStatus) {
        if self.runs.contains_key(run) || self.runs.len() < MAX_TRACKED_RUNS {
            self.runs.insert(run.clone(), status);
        }
    }
    /// The most demanding state of the chat's unfinished runs; without one, whether the
    /// latest finished run failed.
    pub fn activity(&self) -> ChatActivity {
        if self.deleted {
            return ChatActivity::Deleted;
        }
        let has = |wanted: RunStatus| self.runs.values().any(|status| *status == wanted);
        if !self.approvals.is_empty() || has(RunStatus::WaitingApproval) {
            ChatActivity::WaitingApproval
        } else if has(RunStatus::NeedsGrant) {
            ChatActivity::NeedsGrant
        } else if has(RunStatus::NeedsProviderAccess) {
            ChatActivity::NeedsProvider
        } else if has(RunStatus::NeedsUnlock) {
            ChatActivity::NeedsSsh
        } else if !self.runs.is_empty() {
            ChatActivity::Running
        } else if self.failed {
            ChatActivity::Failed
        } else {
            ChatActivity::Idle
        }
    }
    pub fn pending_approvals(&self) -> u16 {
        u16::try_from(self.approvals.len()).unwrap_or(u16::MAX)
    }
    /// The first non-empty line of the first accepted prompt, trimmed and cut at
    /// [`MAX_SUMMARY_PROMPT_LINE_BYTES`] on a character boundary.
    pub fn prompt_line(&self) -> Option<&str> {
        self.prompt_line.as_deref()
    }
    /// The model of the latest accepted prompt.
    pub fn model(&self) -> Option<&ModelSelection> {
        self.model.as_ref()
    }
    pub fn created_at(&self) -> Option<Timestamp> {
        self.created_at
    }
    pub fn last_activity(&self) -> Option<Timestamp> {
        self.last_activity
    }
    pub fn deleted(&self) -> bool {
        self.deleted
    }
    /// Keeps times a previous process recorded: its creation time wins, and the later of
    /// the two activities stays.
    fn restore_times(&mut self, created_at: Timestamp, last_activity: Timestamp) {
        self.created_at = Some(created_at);
        self.last_activity = Some(
            self.last_activity
                .map_or(last_activity, |last| last.max(last_activity))
                .max(created_at),
        );
    }
    /// Keeps what a reload from a compacted journal can no longer see.
    fn inherit(&mut self, earlier: &Self) {
        if self.prompt_line.is_none() {
            self.prompt_line.clone_from(&earlier.prompt_line);
        }
        if self.model.is_none() {
            self.model.clone_from(&earlier.model);
        }
        if let (Some(created), Some(last)) = (earlier.created_at, earlier.last_activity) {
            self.restore_times(created, last);
        }
    }
}

fn prompt_line(prompt: &EnginePrompt) -> Option<String> {
    let line = prompt.content().iter().find_map(|part| match part {
        Content::Text(text) => text
            .text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty()),
        _ => None,
    })?;
    let mut end = line.len().min(MAX_SUMMARY_PROMPT_LINE_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Some(line[..end].to_owned())
}

/// What a runner asks its directory to sign and seal as one revision of a chat's summary.
#[derive(Clone, Debug)]
pub struct ChatSummaryDraft {
    stream: StreamBinding,
    revision: u64,
    created_at: Timestamp,
    last_activity: Timestamp,
    prompt_line: Option<String>,
    model: Option<ModelSelection>,
    activity: ChatActivity,
    pending_approvals: u16,
}
impl ChatSummaryDraft {
    fn new(stream: StreamBinding, revision: u64, facts: &ChatSummaryFacts, now: Timestamp) -> Self {
        let created_at = facts.created_at.unwrap_or(now);
        Self {
            stream,
            revision,
            created_at,
            last_activity: facts.last_activity.unwrap_or(created_at).max(created_at),
            prompt_line: facts.prompt_line.clone(),
            model: facts.model.clone(),
            activity: facts.activity(),
            pending_approvals: facts.pending_approvals(),
        }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    /// At least 1, above every revision staged before for this chat.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
    /// Never before [`Self::created_at`].
    pub fn last_activity(&self) -> Timestamp {
        self.last_activity
    }
    pub fn prompt_line(&self) -> Option<&str> {
        self.prompt_line.as_deref()
    }
    pub fn model(&self) -> Option<&ModelSelection> {
        self.model.as_ref()
    }
    pub fn activity(&self) -> ChatActivity {
        self.activity
    }
    pub fn pending_approvals(&self) -> u16 {
        self.pending_approvals
    }
}

/// One signed and sealed summary revision, opaque to the runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedChatSummary {
    stream: StreamBinding,
    key_epoch: ChatKeyEpoch,
    revision: u64,
    deleted: bool,
    ciphertext: OpaqueBytes,
}
impl SealedChatSummary {
    pub fn new(
        stream: StreamBinding,
        key_epoch: ChatKeyEpoch,
        revision: u64,
        deleted: bool,
        ciphertext: OpaqueBytes,
    ) -> Result<Self, JournalError> {
        if revision == 0
            || ciphertext.is_empty()
            || ciphertext.len() > MAX_SEALED_CHAT_SUMMARY_BYTES
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            stream,
            key_epoch,
            revision,
            deleted,
            ciphertext,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    /// The chat key generation the summary's context names.
    pub fn key_epoch(&self) -> ChatKeyEpoch {
        self.key_epoch
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether this revision announces the chat's deletion, its last summary.
    pub fn deleted(&self) -> bool {
        self.deleted
    }
    pub fn ciphertext(&self) -> &OpaqueBytes {
        &self.ciphertext
    }
}

/// What the runner journal keeps of a chat's summaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredChatSummary {
    revision: u64,
    published: u64,
    created_at: Timestamp,
    last_activity: Timestamp,
    deleted: bool,
    pending: Option<SealedChatSummary>,
}
impl StoredChatSummary {
    /// `pending` is present exactly while `published` is below `revision`, and is that
    /// revision.
    pub fn new(
        revision: u64,
        published: u64,
        created_at: Timestamp,
        last_activity: Timestamp,
        deleted: bool,
        pending: Option<SealedChatSummary>,
    ) -> Result<Self, JournalError> {
        let pending_matches = match &pending {
            Some(sealed) => published < revision && sealed.revision == revision,
            None => published == revision,
        };
        if revision == 0
            || !pending_matches
            || created_at.as_millis() == 0
            || last_activity < created_at
            || pending
                .as_ref()
                .is_some_and(|sealed| sealed.deleted != deleted)
        {
            return Err(JournalError::Corrupt);
        }
        Ok(Self {
            revision,
            published,
            created_at,
            last_activity,
            deleted,
            pending,
        })
    }
    /// The newest staged revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// The newest revision the hub confirmed.
    pub fn published(&self) -> u64 {
        self.published
    }
    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
    pub fn last_activity(&self) -> Timestamp {
        self.last_activity
    }
    /// Whether the newest staged revision announces the deletion.
    pub fn deleted(&self) -> bool {
        self.deleted
    }
    /// The newest revision while the hub has not confirmed it.
    pub fn pending(&self) -> Option<&SealedChatSummary> {
        self.pending.as_ref()
    }
}

/// The runner journal's summary rows (journal v7). Implementations bind each call to the
/// stream's catalog row and never move a revision backwards.
pub trait ChatSummaryJournal: Send + Sync {
    fn summary(&self, stream: StreamBinding) -> JournalFuture<Option<StoredChatSummary>>;
    /// Records `sealed` as the chat's newest revision before it is published. A revision
    /// that is not above the stored one is a [`JournalError::Conflict`].
    fn stage_summary(
        &self,
        sealed: SealedChatSummary,
        created_at: Timestamp,
        last_activity: Timestamp,
    ) -> JournalFuture<()>;
    /// The hub stored `revision`; a confirmation of an older revision changes nothing.
    fn summary_published(&self, stream: StreamBinding, revision: u64) -> JournalFuture<()>;
}

/// The runner's directory stream on its hub. The adapter owns the summary format, the
/// runner's keys and the transport.
pub trait ChatSummaryDirectory: Send + Sync {
    /// Whether the hub keeps runner directories now. A runner publishes only then.
    fn available(&self) -> bool;
    /// Signs and seals `draft` with the chat's current head.
    fn seal(&self, draft: ChatSummaryDraft) -> JournalFuture<SealedChatSummary>;
    /// Publishes exactly these bytes under the revision's own publication id.
    fn publish(&self, sealed: SealedChatSummary) -> JournalFuture<()>;
}

#[cfg(test)]
mod tests;
