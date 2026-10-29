//! Accepted queued runs and the displayed active run have separate identities.
use super::{MAX_PROJECTION_ENTRIES, ProjectionError};
use crate::{
    authority::RunBinding,
    coordinator::RunStatus,
    identity::{RunEpoch, RunId},
};
use nocterm_ai::conversation::ConversationPhase;
use std::collections::BTreeMap;

pub(super) type RunKey = (RunId, RunEpoch);
pub(super) fn key(run: &RunBinding) -> RunKey {
    (run.run(), run.epoch())
}

#[derive(Clone, Default)]
pub(super) struct ProjectionRuns {
    selected: Option<RunBinding>,
    phases: BTreeMap<RunKey, ConversationPhase>,
}
impl ProjectionRuns {
    pub(super) fn selected(&self) -> Option<&RunBinding> {
        self.selected.as_ref()
    }
    pub(super) fn phase(&self) -> ConversationPhase {
        self.selected
            .as_ref()
            .and_then(|run| self.phases.get(&key(run)))
            .copied()
            .unwrap_or(ConversationPhase::Idle)
    }
    pub(super) fn bytes(&self) -> usize {
        1024usize.saturating_mul(self.phases.len())
    }
    pub(super) fn admit(&mut self, run: &RunBinding) -> Result<(), ProjectionError> {
        if self.phases.contains_key(&key(run)) {
            return Err(ProjectionError::InvalidRecord);
        }
        if self.phases.len() >= MAX_PROJECTION_ENTRIES {
            return Err(ProjectionError::Capacity);
        }
        if self.selected.is_none() || terminal(self.phase()) {
            self.selected = Some(run.clone());
        }
        self.phases.insert(key(run), ConversationPhase::Queued);
        Ok(())
    }
    pub(super) fn require_known(&self, run: &RunBinding) -> Result<(), ProjectionError> {
        self.phases
            .contains_key(&key(run))
            .then_some(())
            .ok_or(ProjectionError::WrongRun)
    }
    pub(super) fn require_current(&self, run: &RunBinding) -> Result<(), ProjectionError> {
        (self.selected.as_ref() == Some(run) && !terminal(self.phase()))
            .then_some(())
            .ok_or(ProjectionError::WrongRun)
    }
    pub(super) fn update(
        &mut self,
        run: &RunBinding,
        next: ConversationPhase,
    ) -> Result<(), ProjectionError> {
        self.require_known(run)?;
        let old = self
            .phases
            .get(&key(run))
            .copied()
            .ok_or(ProjectionError::WrongRun)?;
        if matches!(
            old,
            ConversationPhase::Stopped | ConversationPhase::Completed
        ) && old != next
        {
            return Err(ProjectionError::InvalidRecord);
        }
        if next == ConversationPhase::Running && self.selected.as_ref() != Some(run) {
            if !terminal(self.phase()) {
                return Err(ProjectionError::WrongRun);
            }
            self.selected = Some(run.clone());
        }
        self.phases.insert(key(run), next);
        Ok(())
    }
}
fn terminal(phase: ConversationPhase) -> bool {
    matches!(
        phase,
        ConversationPhase::Completed | ConversationPhase::Stopped | ConversationPhase::Interrupted
    )
}
pub(super) fn phase(status: RunStatus) -> ConversationPhase {
    match status {
        RunStatus::Queued => ConversationPhase::Queued,
        RunStatus::Running => ConversationPhase::Running,
        RunStatus::WaitingApproval => ConversationPhase::WaitingApproval,
        RunStatus::NeedsGrant => ConversationPhase::NeedsGrant,
        RunStatus::NeedsUnlock => ConversationPhase::NeedsUnlock,
        RunStatus::NeedsProviderAccess => ConversationPhase::NeedsProviderAccess,
        RunStatus::OutcomeUnknown => ConversationPhase::OutcomeUnknown,
        RunStatus::Interrupted => ConversationPhase::Interrupted,
        RunStatus::Stopped => ConversationPhase::Stopped,
        RunStatus::Completed => ConversationPhase::Completed,
    }
}
