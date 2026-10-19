//! Semantic revisions are independent from serialization, delivery and physical publication.
use crate::runtime::AgentPersistenceError;
use nocterm_ai::history::SharedChat;
use std::sync::Arc;

#[derive(Clone, Default)]
pub(super) struct PersistenceState {
    inputs: super::metadata::SemanticInputs,
    desired: u64,
    published: u64,
    failure: Option<FailureRecord>,
    candidate: Option<Candidate>,
    history_present: bool,
    draft_changed: bool,
}
#[derive(Clone)]
struct FailureRecord {
    revision: u64,
    error: AgentPersistenceError,
}
#[derive(Clone)]
pub(super) struct Candidate {
    snapshot: Arc<SharedChat>,
    revision: u64,
    policy: CandidatePolicy,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CandidatePolicy {
    Prepared,
    Scheduled,
    Admitted,
    Retry,
}
impl PersistenceState {
    pub(super) fn inputs_match(&self, input: &super::metadata::Inputs<'_>) -> bool {
        self.inputs.matches(input)
    }
    pub(super) fn remember_inputs(&mut self, input: &super::metadata::Inputs<'_>) {
        self.inputs.remember(input);
    }
    pub(super) fn restart_snapshot(&self) -> Self {
        let mut copy = self.clone();
        if let Some(candidate) = &mut copy.candidate
            && candidate.revision > copy.published
        {
            candidate.policy = CandidatePolicy::Prepared;
        }
        copy
    }
    pub(super) fn inputs_ready(&self) -> bool {
        self.inputs.initialized()
    }
    pub(super) fn begin_hydration(&mut self) {
        self.inputs.begin_hydration();
    }
    pub(super) fn fork_created(&mut self) {
        self.desired = 0;
        self.published = 0;
        self.failure = None;
        self.candidate = None;
        self.history_present = false;
        self.draft_changed = false;
        self.changed(false);
    }
    pub(super) fn imported_clean(&mut self) {
        self.history_present = true;
        self.published = self.desired;
        self.failure = None;
        self.candidate = None;
        self.draft_changed = false;
    }
    pub(super) fn changed(&mut self, draft: bool) -> u64 {
        self.desired += 1;
        self.draft_changed |= draft;
        self.failure = None;
        self.desired
    }
    pub(super) fn desired(&self) -> u64 {
        self.desired
    }
    pub(super) fn published_revision(&self) -> u64 {
        self.published
    }
    pub(super) fn failure(&self) -> Option<&AgentPersistenceError> {
        self.failure.as_ref().map(|failure| &failure.error)
    }
    pub(super) fn history_present(&self) -> bool {
        self.history_present
    }
    pub(super) fn draft_changed(&self) -> bool {
        self.draft_changed
    }
    pub(super) fn clean(&self) -> bool {
        self.desired <= self.published && self.failure.is_none()
    }
    pub(super) fn current_snapshot(&self) -> Option<Arc<SharedChat>> {
        self.candidate
            .as_ref()
            .filter(|candidate| candidate.revision == self.desired)
            .map(|candidate| candidate.snapshot.clone())
    }
    pub(super) fn has_current_candidate(&self) -> bool {
        self.candidate
            .as_ref()
            .is_some_and(|candidate| candidate.revision == self.desired)
    }
    pub(super) fn writer_owns_current(&self) -> bool {
        self.candidate.as_ref().is_some_and(|candidate| {
            candidate.revision == self.desired && candidate.policy == CandidatePolicy::Admitted
        })
    }
    pub(super) fn prepare(&mut self, snapshot: Arc<SharedChat>) {
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| candidate.revision == self.desired)
        {
            return;
        }
        self.candidate = Some(Candidate {
            snapshot,
            revision: self.desired,
            policy: CandidatePolicy::Prepared,
        });
    }
    pub(super) fn capture_pending(&self) -> Option<(Arc<SharedChat>, u64)> {
        let candidate = self.candidate.as_ref()?;
        (candidate.revision == self.desired && candidate.policy != CandidatePolicy::Admitted)
            .then(|| (candidate.snapshot.clone(), candidate.revision))
    }
    pub(super) fn mark_normal_offer(&mut self, revision: u64) -> bool {
        let Some(candidate) = &mut self.candidate else {
            return false;
        };
        if candidate.revision != revision
            || !matches!(
                candidate.policy,
                CandidatePolicy::Prepared | CandidatePolicy::Retry
            )
        {
            return false;
        }
        candidate.policy = CandidatePolicy::Scheduled;
        true
    }
    pub(super) fn acknowledge_admission(&mut self, revision: u64) {
        if let Some(candidate) = &mut self.candidate
            && candidate.revision == revision
        {
            candidate.policy = CandidatePolicy::Admitted;
            self.history_present = true;
            if revision == self.desired {
                self.draft_changed = false;
            }
        }
    }
    pub(super) fn published(&mut self, revision: u64) -> bool {
        if revision > self.desired {
            return false;
        }
        self.published = self.published.max(revision);
        if self
            .failure
            .as_ref()
            .is_some_and(|failure| revision >= failure.revision)
        {
            self.failure = None;
        }
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| revision >= candidate.revision)
        {
            self.candidate = None;
        }
        self.clean()
    }
    pub(super) fn failed(&mut self, revision: u64, error: AgentPersistenceError) -> bool {
        if revision != self.desired || revision <= self.published {
            return false;
        }
        self.failure = Some(FailureRecord { revision, error });
        if let Some(candidate) = &mut self.candidate
            && candidate.revision == revision
        {
            candidate.policy = CandidatePolicy::Retry;
        }
        true
    }
}
