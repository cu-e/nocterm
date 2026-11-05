//! The revision ledger and publication cadence of one chat's summary.
use super::{
    ChatSummaryDraft, ChatSummaryFacts, SUMMARY_INTERVAL_MS, SUMMARY_RETRY_CEILING_MS,
    SealedChatSummary, StoredChatSummary, SummaryObservation,
};
use crate::{authority::StreamBinding, identity::Timestamp, journal::JournalError};

/// What the runner does next for one chat.
#[derive(Clone, Debug)]
pub enum SummaryStep {
    /// Nothing is due.
    Idle,
    /// The facts changed: sign and seal this new revision, stage it, then publish it.
    Seal(ChatSummaryDraft),
    /// Publish the staged revision again, with the same bytes.
    Publish(SealedChatSummary),
}

/// One chat's facts, its newest staged and published revisions and when to publish next.
///
/// Publication is coalesced: at most one attempt per [`SUMMARY_INTERVAL_MS`], growing to
/// [`SUMMARY_RETRY_CEILING_MS`] after failures; the first attempt after a deletion is
/// immediate. A staged revision that the facts have overtaken is replaced by a higher
/// revision rather than published; a revision never repeats with other bytes.
#[derive(Clone, Debug)]
pub struct ChatSummaryTracker {
    stream: StreamBinding,
    facts: ChatSummaryFacts,
    revision: u64,
    published: u64,
    pending: Option<SealedChatSummary>,
    changed: bool,
    last_attempt: Option<Timestamp>,
    failures: u32,
}
impl ChatSummaryTracker {
    /// Restores a chat from facts derived from its journal and the journal's summary row.
    /// Without a row the chat is backfilled: its first revision is due at once. With one,
    /// the facts are published again once, since the process that stopped may not have
    /// published its last change; a published deletion stays final.
    pub fn restore(
        stream: StreamBinding,
        mut facts: ChatSummaryFacts,
        stored: Option<StoredChatSummary>,
    ) -> Result<Self, JournalError> {
        let Some(stored) = stored else {
            return Ok(Self::new(stream, facts, 0, 0, None, true));
        };
        if stored
            .pending()
            .is_some_and(|sealed| sealed.stream() != &stream)
        {
            return Err(JournalError::Corrupt);
        }
        facts.restore_times(stored.created_at(), stored.last_activity());
        let finished = stored.deleted() && stored.pending().is_none();
        Ok(Self::new(
            stream,
            facts,
            stored.revision(),
            stored.published(),
            stored.pending().cloned(),
            !finished,
        ))
    }
    fn new(
        stream: StreamBinding,
        facts: ChatSummaryFacts,
        revision: u64,
        published: u64,
        pending: Option<SealedChatSummary>,
        changed: bool,
    ) -> Self {
        Self {
            stream,
            facts,
            revision,
            published,
            pending,
            changed,
            last_attempt: None,
            failures: 0,
        }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn facts(&self) -> &ChatSummaryFacts {
        &self.facts
    }
    pub fn observe(&mut self, observation: SummaryObservation<'_>, at: Timestamp) {
        if !self.finished() {
            self.facts.observe(observation, at);
            self.changed = true;
        }
    }
    /// Takes the SSH binding and access the runner holds for the chat now; a change makes a
    /// new revision due (ADR 0147).
    pub fn observe_ssh(&mut self, ssh: Option<super::SummarySsh>) {
        if !self.finished() && self.facts.observe_ssh(ssh) {
            self.changed = true;
        }
    }
    /// Replaces the facts after a reload from a journal whose earlier records are gone,
    /// keeping the title, model and times the earlier facts already held.
    pub fn reload(&mut self, mut facts: ChatSummaryFacts) {
        if self.finished() {
            return;
        }
        facts.inherit(&self.facts);
        self.facts = facts;
        self.changed = true;
    }
    pub fn next(&self, now: Timestamp) -> SummaryStep {
        if self.finished() || !self.due(now) {
            return SummaryStep::Idle;
        }
        if self.changed {
            let revision = self.revision.max(self.published).saturating_add(1);
            return SummaryStep::Seal(ChatSummaryDraft::new(
                self.stream.clone(),
                revision,
                &self.facts,
                now,
            ));
        }
        match &self.pending {
            Some(sealed) => SummaryStep::Publish(sealed.clone()),
            None => SummaryStep::Idle,
        }
    }
    fn due(&self, now: Timestamp) -> bool {
        let Some(last) = self.last_attempt else {
            return true;
        };
        if self.failures == 0 && self.facts.deleted() && self.changed {
            return true;
        }
        let wait = if self.failures == 0 {
            SUMMARY_INTERVAL_MS
        } else {
            SUMMARY_INTERVAL_MS
                .saturating_mul(1 << self.failures.min(8))
                .min(SUMMARY_RETRY_CEILING_MS)
        };
        now.elapsed_since(last)
            .is_some_and(|elapsed| elapsed >= wait)
            || now < last
    }
    /// The journal staged `sealed`, the revision [`Self::next`] drafted.
    pub fn staged(&mut self, sealed: SealedChatSummary) -> Result<(), JournalError> {
        let expected = self.revision.max(self.published).saturating_add(1);
        if sealed.stream() != &self.stream
            || sealed.revision() != expected
            || sealed.deleted() != self.facts.deleted()
        {
            return Err(JournalError::Conflict);
        }
        self.revision = sealed.revision();
        self.pending = Some(sealed);
        self.changed = false;
        Ok(())
    }
    /// The hub stored `revision`.
    pub fn published(&mut self, revision: u64, now: Timestamp) {
        self.published = self.published.max(revision);
        if self
            .pending
            .as_ref()
            .is_some_and(|sealed| sealed.revision() <= revision)
        {
            self.pending = None;
        }
        self.last_attempt = Some(now);
        self.failures = 0;
    }
    /// An attempt failed; the next one waits longer.
    pub fn failed(&mut self, now: Timestamp) {
        self.last_attempt = Some(now);
        self.failures = self.failures.saturating_add(1);
    }
    /// The chat's deletion is published: nothing follows it.
    pub fn finished(&self) -> bool {
        self.facts.deleted() && !self.changed && self.pending.is_none() && self.revision > 0
    }
}
