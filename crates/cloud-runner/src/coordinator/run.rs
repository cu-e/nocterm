//! Foreground run state; background effects remain owned by the durable journal.
use super::{control::RunControl, frame::Frame, log::ChatLog};
use futures::{channel::oneshot, lock::Mutex};
use nocterm_cloud_model::{
    authority::*, coordinator::*, grant::*, identity::*, inference::*, journal::*, run::ToolIntent,
};
use parking_lot::Mutex as SyncMutex;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct PendingApproval {
    intent: ToolIntent,
    expires: Timestamp,
    answer: Option<oneshot::Sender<ApprovalChoice>>,
    armed: bool,
}
impl PendingApproval {
    pub(super) fn new(
        intent: ToolIntent,
        expires: Timestamp,
        answer: oneshot::Sender<ApprovalChoice>,
    ) -> Self {
        Self {
            intent,
            expires,
            answer: Some(answer),
            armed: false,
        }
    }
    pub(super) fn matches(&self, answer: &ApprovalAnswer, now: Timestamp) -> bool {
        self.armed
            && self.intent.id() == answer.intent()
            && self.intent.authority() == answer.authority()
            && self.intent.arguments() == answer.arguments()
            && now < self.expires
            && self.answer.is_some()
    }
    pub(super) fn resolve(&mut self, choice: ApprovalChoice) -> Result<(), CoordinatorError> {
        self.answer
            .take()
            .ok_or(CoordinatorError::Conflict)?
            .send(choice)
            .map_err(|_| CoordinatorError::Stopped)
    }
}
#[derive(Clone, Default)]
pub(super) struct Grants {
    inference: Option<InferenceGrant>,
    tools: BTreeMap<GrantId, GrantState>,
    revoked: BTreeMap<GrantId, GrantEpoch>,
}
impl Grants {
    pub(super) fn inference(&self) -> Option<&InferenceGrant> {
        self.inference.as_ref().filter(|grant| {
            self.revoked
                .get(&grant.id())
                .is_none_or(|old| *old < grant.epoch())
        })
    }
    pub(super) fn install_inference(
        &mut self,
        grant: InferenceGrant,
    ) -> Result<(), CoordinatorError> {
        if self
            .revoked
            .get(&grant.id())
            .is_some_and(|epoch| *epoch >= grant.epoch())
            || self
                .inference
                .as_ref()
                .is_some_and(|old| old.id() == grant.id() && old.epoch() >= grant.epoch())
        {
            return Err(CoordinatorError::Conflict);
        }
        self.inference = Some(grant);
        Ok(())
    }
    pub(super) fn install_tool(&mut self, policy: GrantPolicy) -> Result<(), CoordinatorError> {
        let id = policy.authority().grant();
        let epoch = policy.authority().grant_epoch();
        if self.revoked.get(&id).is_some_and(|old| *old >= epoch)
            || self
                .tools
                .get(&id)
                .is_some_and(|old| old.authority().grant_epoch() >= epoch)
        {
            return Err(CoordinatorError::Conflict);
        }
        let state = GrantState::from_snapshot(GrantSnapshot::new(
            policy.clone(),
            policy.operations(),
            policy.window().issued(),
            None,
        )?);
        self.tools.insert(id, state);
        Ok(())
    }
    pub(super) fn revoke(&mut self, reference: GrantReference) {
        self.revoked
            .entry(reference.id())
            .and_modify(|old| *old = (*old).max(reference.epoch()))
            .or_insert(reference.epoch());
        if let Some(tool) = self.tools.get_mut(&reference.id()) {
            tool.revoke();
        }
    }
    pub(super) fn tool(&self, id: GrantId) -> Option<&GrantState> {
        self.tools.get(&id)
    }
    pub(super) fn replace_tool(&mut self, state: GrantState) {
        self.tools.insert(state.authority().grant(), state);
    }
    pub(super) fn select_tool(
        &self,
        session: &SessionReceipt,
        call: &nocterm_ai::TerminalCall,
        now: Timestamp,
    ) -> Option<GrantState> {
        self.tools
            .values()
            .find(|state| {
                let mut preview = (*state).clone();
                state.authority().session() == session
                    && preview.authorize(state.authority(), call, now).is_ok()
            })
            .cloned()
    }
    pub(super) fn revoke_issuer(&mut self, revoke: &IssuerRevocation) {
        if let Some(grant) = &self.inference {
            if revoked_origin(grant.origin(), revoke) {
                self.revoked.insert(grant.id(), grant.epoch());
            }
        }
        for state in self.tools.values_mut() {
            if revoked_origin(state.origin(), revoke) {
                state.revoke();
            }
        }
        for reference in revoke.grants() {
            self.revoke(*reference);
        }
    }
}
fn revoked_origin(origin: &GrantOrigin, revoke: &IssuerRevocation) -> bool {
    origin.issuer().device() == revoke.issuer().device()
        && origin.issuer().key() == revoke.issuer().key()
        && origin.control() <= revoke.cutoff()
}

pub(super) struct RunData {
    ledger: InferenceLedger,
    status: RunStatus,
}
impl RunData {
    pub(super) fn ledger(&self) -> &InferenceLedger {
        &self.ledger
    }
    pub(super) fn set_ledger(&mut self, ledger: InferenceLedger) {
        self.ledger = ledger;
    }
    pub(super) fn status(&self) -> RunStatus {
        self.status
    }
    pub(super) fn set_status(&mut self, status: RunStatus) {
        self.status = status;
    }
}
pub(super) struct RunRecord {
    binding: RunBinding,
    context: super::run_context::RunContext,
    control: Arc<RunControl>,
    state: Mutex<RunData>,
    approval: SyncMutex<Option<PendingApproval>>,
    grants: SyncMutex<Grants>,
    recovery: SyncMutex<Option<super::recovery::RecoveryPlan>>,
    wake: Arc<AtomicBool>,
    sessions: Arc<super::run_session::RunSessionOwner>,
}
impl RunRecord {
    pub(super) fn new(
        binding: RunBinding,
        prompt: PromptRequest,
    ) -> Result<Self, CoordinatorError> {
        let allowance = prompt.allowance();
        let task = InferenceTask::new(
            binding.clone(),
            prompt.selection().clone(),
            allowance.budget(),
            allowance.spend_micros(),
            allowance.window(),
        )?;
        let wake = Arc::new(AtomicBool::new(false));
        let sessions = super::run_session::RunSessionOwner::new(
            binding.clone(),
            prompt.destination().target().clone(),
            prompt.destination().authentication().clone(),
            wake.clone(),
        );
        Ok(Self {
            binding,
            context: (&prompt).into(),
            control: Arc::new(RunControl::default()),
            state: Mutex::new(RunData {
                ledger: InferenceLedger::new(task),
                status: RunStatus::Queued,
            }),
            approval: SyncMutex::new(None),
            grants: SyncMutex::new(Grants::default()),
            recovery: SyncMutex::new(None),
            wake,
            sessions,
        })
    }
    pub(super) fn binding(&self) -> &RunBinding {
        &self.binding
    }
    pub(super) fn sessions(&self) -> &Arc<super::run_session::RunSessionOwner> {
        &self.sessions
    }
    pub(super) fn selected_session(&self) -> Option<SessionReceipt> {
        if let Some(fact) = self.sessions.live() {
            if let SessionOutcome::Established(receipt) = fact.value().outcome() {
                return Some(receipt.clone());
            }
        }
        self.sessions
            .permits_initial_reference()
            .then(|| self.context.destination().session().cloned())
            .flatten()
    }
    pub(super) fn context(&self) -> &super::run_context::RunContext {
        &self.context
    }
    pub(super) fn control(&self) -> &Arc<RunControl> {
        &self.control
    }
    pub(super) fn grants(&self) -> Grants {
        self.grants.lock().clone()
    }
    pub(super) fn set_grants(&self, grants: Grants) {
        *self.grants.lock() = grants;
    }
    pub(super) fn set_recovery(&self, recovery: Option<super::recovery::RecoveryPlan>) {
        *self.recovery.lock() = recovery;
    }
    pub(super) fn take_recovery(&self) -> Option<super::recovery::RecoveryPlan> {
        self.recovery.lock().take()
    }
    pub(super) fn wake(&self) {
        self.wake.store(true, Ordering::Release);
    }
    pub(super) fn wants_resume(&self) -> bool {
        self.wake.load(Ordering::Acquire)
    }
    pub(super) fn resumed(&self) {
        self.wake.store(false, Ordering::Release);
    }
    pub(super) fn state(&self) -> &Mutex<RunData> {
        &self.state
    }
    pub(super) fn install_approval(
        &self,
        approval: PendingApproval,
    ) -> Result<(), CoordinatorError> {
        let mut pending = self.approval.lock();
        if pending.is_some() {
            return Err(CoordinatorError::Conflict);
        }
        *pending = Some(approval);
        Ok(())
    }
    pub(super) fn arm_approval(&self) {
        if let Some(approval) = self.approval.lock().as_mut() {
            approval.armed = true;
        }
    }
    pub(super) fn check_approval(
        &self,
        answer: &ApprovalAnswer,
        now: Timestamp,
    ) -> Result<(), CoordinatorError> {
        if self.selected_session().as_ref() != Some(answer.authority().session()) {
            return Err(CoordinatorError::NeedsUnlock);
        }
        self.control.require_runtime_current()?;
        if self
            .approval
            .lock()
            .as_ref()
            .is_some_and(|pending| pending.matches(answer, now))
        {
            Ok(())
        } else {
            Err(CoordinatorError::Conflict)
        }
    }
    pub(super) fn resolve_approval(&self, answer: &ApprovalAnswer) -> Result<(), CoordinatorError> {
        let mut pending = self
            .approval
            .lock()
            .take()
            .ok_or(CoordinatorError::Conflict)?;
        pending.resolve(answer.choice())
    }
    pub(super) fn invalidate_session_approval(&self) {
        let previous = self.approval.lock().take();
        drop(previous);
    }
    pub(super) async fn status(
        &self,
        log: &ChatLog,
        status: RunStatus,
    ) -> Result<(), CoordinatorError> {
        self.status_with(log, status, None).await
    }
    pub(super) async fn status_after_command(
        &self,
        log: &ChatLog,
        status: RunStatus,
        original: &CommandReceipt,
    ) -> Result<(), CoordinatorError> {
        self.status_with(log, status, Some(original)).await
    }
    async fn status_with(
        &self,
        log: &ChatLog,
        status: RunStatus,
        original: Option<&CommandReceipt>,
    ) -> Result<(), CoordinatorError> {
        let mut state = self.state.lock().await;
        let record = JournalRecord::new(
            RecordContext::Run(self.binding.clone()),
            JournalRecordKind::RunStatus,
            Frame::Status(status_tag(status)).bytes()?,
        )?;
        let records = vec![(
            record,
            Some(CoordinatorEvent::Status {
                run: self.binding.clone(),
                status,
            }),
        )];
        if let Some(original) = original {
            log.append_after_command(original, records).await?;
        } else {
            log.append(None, records, None).await?;
        }
        state.status = status;
        Ok(())
    }
}
pub(super) fn status_tag(status: RunStatus) -> u8 {
    match status {
        RunStatus::Queued => 0,
        RunStatus::Running => 1,
        RunStatus::WaitingApproval => 2,
        RunStatus::NeedsGrant => 3,
        RunStatus::NeedsUnlock => 4,
        RunStatus::NeedsProviderAccess => 5,
        RunStatus::OutcomeUnknown => 6,
        RunStatus::Interrupted => 7,
        RunStatus::Stopped => 8,
        RunStatus::Completed => 9,
    }
}
