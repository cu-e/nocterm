//! Host-controlled durable-effect ordering; no transition performs an effect.
use crate::{
    authority::{AuthorityError, ExecutionAuthority, RunBinding, TargetBinding},
    identity::{CommandId, Fingerprint, IdentityError, IntentId, Revision, StepId, WriterEpoch},
};
use nocterm_ai::{TerminalCall, TerminalCallError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolIntent {
    id: IntentId,
    step: StepId,
    authority: ExecutionAuthority,
    arguments: Fingerprint,
    mutates: bool,
}
impl ToolIntent {
    /// Restore an authenticated native journal's descriptive intent. This does
    /// not validate a model call, admit execution or produce a dispatch permit.
    pub fn from_journal(
        id: IntentId,
        step: StepId,
        authority: ExecutionAuthority,
        arguments: Fingerprint,
        mutates: bool,
    ) -> Self {
        Self {
            id,
            step,
            authority,
            arguments,
            mutates,
        }
    }
    /// The host allocates identity and supplies the digest of the exact validated arguments.
    pub fn new(
        id: IntentId,
        step: StepId,
        authority: ExecutionAuthority,
        arguments: Fingerprint,
        call: &TerminalCall,
    ) -> Result<Self, RunError> {
        call.validate()?;
        Ok(Self {
            id,
            step,
            authority,
            arguments,
            mutates: call.writes(),
        })
    }
    pub fn id(&self) -> IntentId {
        self.id
    }
    pub fn step(&self) -> StepId {
        self.step
    }
    pub fn authority(&self) -> &ExecutionAuthority {
        &self.authority
    }
    pub fn arguments(&self) -> Fingerprint {
        self.arguments
    }
    pub fn mutates(&self) -> bool {
        self.mutates
    }
}

/// A journal transaction observation. Construction does not prove persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitStamp {
    run: RunBinding,
    writer: WriterEpoch,
    revision: Revision,
}
impl CommitStamp {
    pub fn new(run: RunBinding, writer: WriterEpoch, revision: Revision) -> Self {
        Self {
            run,
            writer,
            revision,
        }
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn writer(&self) -> WriterEpoch {
        self.writer
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

/// Only a committed dispatch transition produces this description of the released effect.
/// The native executor must still revalidate its grant/session immediately before I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchPermit {
    intent: ToolIntent,
    commit: CommitStamp,
}
impl DispatchPermit {
    pub fn intent(&self) -> &ToolIntent {
        &self.intent
    }
    pub fn commit(&self) -> &CommitStamp {
        &self.commit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectStage {
    Prepared,
    IntentCommitted,
    DispatchCommitted,
}
#[derive(Clone, Debug)]
struct PendingEffect {
    intent: ToolIntent,
    stage: EffectStage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownEffect {
    intent: ToolIntent,
}
impl UnknownEffect {
    pub fn intent(&self) -> &ToolIntent {
        &self.intent
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectOutcome {
    Known(Fingerprint),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPhase {
    Running,
    OutcomeUnknown,
    Interrupted,
    Completed,
}

/// A durable verified control result supplied by the host, never by model metadata.
#[derive(Clone, Debug)]
pub struct ReconciliationReceipt {
    pub command: CommandId,
    pub commit: CommitStamp,
    pub original: UnknownEffect,
    pub decision: ReconciliationDecision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationDecision {
    Observed(Fingerprint),
    RetryAuthorized,
}

/// Last durable effect information supplied by journal replay after worker loss.
#[derive(Clone, Debug)]
pub struct RecoveredEffect {
    pub intent: ToolIntent,
    pub stage: EffectStage,
}

/// Replay retains unresolved mutations independently from the last pending read.
#[derive(Clone, Debug, Default)]
pub struct RecoveredRunEffects {
    pub pending: Option<RecoveredEffect>,
    pub unresolved: Option<ToolIntent>,
}

/// The host invokes commit transitions only after the matching journal ACK.
/// SDK history and an in-memory state change are never that ACK.
#[derive(Clone, Debug)]
pub struct RunState {
    identity: RunBinding,
    target: TargetBinding,
    writer: WriterEpoch,
    revision: Revision,
    phase: RunPhase,
    pending: Option<PendingEffect>,
    unknown: Option<UnknownEffect>,
}
impl RunState {
    /// Describes an already admitted run; this constructor does not admit a command.
    pub fn new(
        identity: RunBinding,
        target: TargetBinding,
        writer: WriterEpoch,
        revision: Revision,
    ) -> Self {
        Self {
            identity,
            target,
            writer,
            revision,
            phase: RunPhase::Running,
            pending: None,
            unknown: None,
        }
    }
    pub fn identity(&self) -> &RunBinding {
        &self.identity
    }
    pub fn target(&self) -> &TargetBinding {
        &self.target
    }
    pub fn writer(&self) -> WriterEpoch {
        self.writer
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub fn phase(&self) -> RunPhase {
        self.phase
    }
    pub fn unknown(&self) -> Option<&UnknownEffect> {
        self.unknown.as_ref()
    }
    pub fn pending_stage(&self) -> Option<EffectStage> {
        self.pending.as_ref().map(|p| p.stage)
    }

    pub fn prepare_intent(
        &mut self,
        intent: ToolIntent,
        expected: Revision,
    ) -> Result<(), RunError> {
        self.require_active()?;
        self.require_intent(&intent)?;
        if expected != self.revision {
            return Err(RunError::Conflict);
        }
        self.revision.next()?;
        if self.pending.is_some() {
            return Err(RunError::EffectPending);
        }
        if self.unknown.is_some() && intent.mutates {
            return Err(RunError::ReconciliationRequired);
        }
        self.pending = Some(PendingEffect {
            intent,
            stage: EffectStage::Prepared,
        });
        Ok(())
    }

    pub fn intent_committed(&mut self, id: IntentId, stamp: CommitStamp) -> Result<(), RunError> {
        self.require_commit(&stamp)?;
        self.require_pending(id, EffectStage::Prepared)?;
        if let Some(pending) = &mut self.pending {
            pending.stage = EffectStage::IntentCommitted;
        }
        self.revision = stamp.revision;
        Ok(())
    }

    pub fn dispatch_committed(
        &mut self,
        id: IntentId,
        stamp: CommitStamp,
    ) -> Result<DispatchPermit, RunError> {
        self.require_commit(&stamp)?;
        let intent = self
            .require_pending(id, EffectStage::IntentCommitted)?
            .intent
            .clone();
        if let Some(pending) = &mut self.pending {
            pending.stage = EffectStage::DispatchCommitted;
        }
        self.revision = stamp.revision;
        Ok(DispatchPermit {
            intent,
            commit: stamp,
        })
    }

    /// A known non-effect decision before dispatch, never a fabricated dispatch.
    pub fn decline_committed(&mut self, id: IntentId, stamp: CommitStamp) -> Result<(), RunError> {
        self.require_commit(&stamp)?;
        self.require_pending(id, EffectStage::IntentCommitted)?;
        self.pending = None;
        self.revision = stamp.revision;
        Ok(())
    }

    pub fn outcome_committed(
        &mut self,
        id: IntentId,
        stamp: CommitStamp,
        outcome: EffectOutcome,
    ) -> Result<(), RunError> {
        self.require_commit(&stamp)?;
        let intent = self
            .require_pending(id, EffectStage::DispatchCommitted)?
            .intent
            .clone();
        if outcome == EffectOutcome::Unknown && intent.mutates {
            self.unknown = Some(UnknownEffect { intent });
            self.phase = RunPhase::OutcomeUnknown;
        }
        self.pending = None;
        self.revision = stamp.revision;
        Ok(())
    }

    pub fn reconcile(&mut self, receipt: ReconciliationReceipt) -> Result<(), RunError> {
        self.require_active()?;
        self.require_commit(&receipt.commit)?;
        if self.pending.is_some() {
            return Err(RunError::EffectPending);
        }
        if self.unknown.as_ref() != Some(&receipt.original) {
            return Err(RunError::StaleReconciliation);
        }
        self.unknown = None;
        self.phase = RunPhase::Running;
        self.revision = receipt.commit.revision;
        Ok(())
    }

    pub fn interrupt(&mut self, stamp: CommitStamp) -> Result<(), RunError> {
        self.require_commit(&stamp)?;
        if matches!(self.phase, RunPhase::Completed | RunPhase::Interrupted) {
            return Err(RunError::WrongPhase);
        }
        self.capture_uncertain();
        self.phase = RunPhase::Interrupted;
        self.revision = stamp.revision;
        Ok(())
    }

    /// A persisted new run epoch fences every completion captured before interruption.
    /// It never clears an unresolved mutation or changes the selected runner/target.
    pub fn resume(&mut self, stamp: CommitStamp) -> Result<(), RunError> {
        if self.phase != RunPhase::Interrupted {
            return Err(RunError::WrongPhase);
        }
        if !self.identity.same_run(&stamp.run)
            || stamp.run.epoch() != self.identity.epoch().next()?
        {
            return Err(AuthorityError::StaleRun.into());
        }
        if stamp.writer < self.writer {
            return Err(RunError::Fenced);
        }
        if stamp.revision != self.revision.next()? {
            return Err(RunError::Conflict);
        }
        self.identity = stamp.run;
        self.writer = stamp.writer;
        self.revision = stamp.revision;
        self.phase = if self.unknown.is_some() {
            RunPhase::OutcomeUnknown
        } else {
            RunPhase::Running
        };
        Ok(())
    }

    pub fn complete(&mut self, stamp: CommitStamp) -> Result<(), RunError> {
        self.require_active()?;
        self.require_commit(&stamp)?;
        if self.pending.is_some() {
            return Err(RunError::EffectPending);
        }
        if self.unknown.is_some() {
            return Err(RunError::ReconciliationRequired);
        }
        self.phase = RunPhase::Completed;
        self.revision = stamp.revision;
        Ok(())
    }

    /// Replay never resubmits an effect. A lost worker is interrupted; lost SSH
    /// resources additionally require the session owner's explicit reauthorization.
    pub fn recover(
        identity: RunBinding,
        target: TargetBinding,
        writer: WriterEpoch,
        revision: Revision,
        effects: RecoveredRunEffects,
    ) -> Result<Self, RunError> {
        let mut state = Self::new(identity, target, writer, revision);
        if let Some(intent) = effects.unresolved {
            state.require_historical_mutation(&intent)?;
            state.unknown = Some(UnknownEffect { intent });
        }
        if let Some(effect) = effects.pending {
            state.require_intent(&effect.intent)?;
            if effect.stage == EffectStage::DispatchCommitted && effect.intent.mutates {
                if state
                    .unknown
                    .as_ref()
                    .is_some_and(|unknown| unknown.intent != effect.intent)
                {
                    return Err(RunError::InvalidRecovery);
                }
                state.unknown = Some(UnknownEffect {
                    intent: effect.intent,
                });
            }
        }
        state.phase = RunPhase::Interrupted;
        Ok(state)
    }

    /// Checked descriptive recovery of a durable intent that was never dispatched.
    /// The journal owner must acquire/check the supplied new writer first. No
    /// DispatchPermit is restored: current source, grant and original session
    /// still gate a later durable dispatch and native I/O.
    pub fn recover_undispatched(
        identity: RunBinding,
        target: TargetBinding,
        writer: WriterEpoch,
        revision: Revision,
        intent: ToolIntent,
    ) -> Result<Self, RunError> {
        if revision.get() == 0 {
            return Err(RunError::InvalidRecovery);
        }
        let mut state = Self::new(identity, target, writer, revision);
        state.require_intent(&intent)?;
        state.pending = Some(PendingEffect {
            intent,
            stage: EffectStage::IntentCommitted,
        });
        Ok(state)
    }

    fn require_historical_mutation(&self, intent: &ToolIntent) -> Result<(), RunError> {
        let original = intent.authority.run();
        if original.scope() != self.identity.scope() {
            return Err(AuthorityError::WrongScope.into());
        }
        if !original.same_run(&self.identity) || original.epoch() > self.identity.epoch() {
            return Err(AuthorityError::StaleRun.into());
        }
        if intent.authority.session().target() != &self.target {
            return Err(AuthorityError::WrongTarget.into());
        }
        if !intent.mutates {
            return Err(RunError::InvalidRecovery);
        }
        Ok(())
    }
    fn require_active(&self) -> Result<(), RunError> {
        match self.phase {
            RunPhase::Running | RunPhase::OutcomeUnknown => Ok(()),
            RunPhase::Interrupted | RunPhase::Completed => Err(RunError::WrongPhase),
        }
    }
    fn require_intent(&self, intent: &ToolIntent) -> Result<(), RunError> {
        if intent.authority.run().scope() != self.identity.scope() {
            return Err(AuthorityError::WrongScope.into());
        }
        if intent.authority.run() != &self.identity {
            return Err(AuthorityError::StaleRun.into());
        }
        if intent.authority.session().target() != &self.target {
            return Err(AuthorityError::WrongTarget.into());
        }
        Ok(())
    }
    fn require_commit(&self, stamp: &CommitStamp) -> Result<(), RunError> {
        if stamp.writer != self.writer {
            return Err(RunError::Fenced);
        }
        if stamp.run != self.identity {
            return Err(AuthorityError::StaleRun.into());
        }
        if stamp.revision != self.revision.next()? {
            return Err(RunError::Conflict);
        }
        Ok(())
    }
    fn require_pending(
        &self,
        id: IntentId,
        stage: EffectStage,
    ) -> Result<&PendingEffect, RunError> {
        let pending = self.pending.as_ref().ok_or(RunError::StaleIntent)?;
        if pending.intent.id != id {
            return Err(RunError::StaleIntent);
        }
        if pending.stage != stage {
            return Err(RunError::WrongEffectStage);
        }
        Ok(pending)
    }
    fn capture_uncertain(&mut self) {
        if let Some(pending) = self.pending.take()
            && pending.stage == EffectStage::DispatchCommitted
            && pending.intent.mutates
        {
            self.unknown = Some(UnknownEffect {
                intent: pending.intent,
            });
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum RunError {
    #[error("the original execution authority does not match")]
    Authority(#[from] AuthorityError),
    #[error("the run identity or revision is invalid")]
    InvalidIdentity(#[from] IdentityError),
    #[error("the tool arguments are invalid")]
    InvalidArguments(#[from] TerminalCallError),
    #[error("the journal writer epoch is fenced")]
    Fenced,
    #[error("the expected journal revision does not match")]
    Conflict,
    #[error("the run does not admit this transition")]
    WrongPhase,
    #[error("another tool effect is pending")]
    EffectPending,
    #[error("the original tool intent no longer matches")]
    StaleIntent,
    #[error("the tool effect has not reached its required durable stage")]
    WrongEffectStage,
    #[error("the uncertain mutation requires explicit original-intent reconciliation")]
    ReconciliationRequired,
    #[error("the recovered effects contain inconsistent unresolved mutations")]
    InvalidRecovery,
    #[error("the original uncertain intent does not match the reconciliation")]
    StaleReconciliation,
}
