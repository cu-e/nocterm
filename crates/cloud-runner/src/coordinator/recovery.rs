//! Bounded authenticated journal replay. No replay path repeats dispatched I/O.
use super::{
    chat::Chat,
    frame::{Frame, PlannedCall},
    queue::RunQueue,
    run::RunRecord,
    services::CoordinatorServices,
};
use nocterm_cloud_model::{
    admission::{QueuedRun, RunAccess},
    authority::RunBinding,
    coordinator::*,
    engine::*,
    identity::*,
    journal::*,
    payload::BoundedJson,
    run::*,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub(super) struct RecoveryPlan {
    state: RunState,
    step: StepId,
    calls: Vec<PlannedCall>,
    known: BTreeMap<IntentId, EngineToolResult>,
    pending: Option<ToolIntent>,
    choice: Option<ApprovalChoice>,
}
impl RecoveryPlan {
    pub(super) fn state(&self) -> RunState {
        self.state.clone()
    }
    pub(super) fn missing(&self) -> Result<Vec<(IntentId, EngineToolCall)>, CoordinatorError> {
        self.calls
            .iter()
            .filter(|call| !self.known.contains_key(&call.intent()))
            .map(|call| Ok((call.intent(), call.restore(self.step)?)))
            .collect()
    }
    pub(super) fn pending(&self) -> Option<&ToolIntent> {
        self.pending.as_ref()
    }
    pub(super) fn choice(&self) -> Option<ApprovalChoice> {
        self.choice
    }
}
struct ReplayedRun {
    record: Arc<RunRecord>,
    revision: Revision,
    status: RunStatus,
    action: Option<(StepId, Vec<PlannedCall>)>,
    known: BTreeMap<IntentId, EngineToolResult>,
    pending: Option<(ToolIntent, EffectStage)>,
    choice: Option<ApprovalChoice>,
    unknown: bool,
    unfinalized: bool,
    declared: bool,
}
impl ReplayedRun {
    fn new(record: Arc<RunRecord>) -> Self {
        Self {
            record,
            revision: Revision::default(),
            status: RunStatus::Queued,
            action: None,
            known: BTreeMap::new(),
            pending: None,
            choice: None,
            unknown: false,
            unfinalized: false,
            declared: false,
        }
    }
    async fn apply(&mut self, stored: &StoredRecord, frame: Frame) -> Result<(), CoordinatorError> {
        if let Some(stamp) = stored.record().context().stamp() {
            if stamp.run() != self.record.binding() || stamp.revision() != self.revision.next()? {
                return Err(JournalError::Corrupt.into());
            }
            self.revision = stamp.revision();
        }
        match (stored.record().kind(), frame) {
            (JournalRecordKind::BudgetReserved { .. }, Frame::Budget(ledger)) => {
                let ledger = ledger.restore()?;
                if ledger.task().run() != self.record.binding() {
                    return Err(JournalError::Corrupt.into());
                }
                self.unfinalized = ledger
                    .snapshot()
                    .calls()
                    .iter()
                    .any(|call| !call.finalized());
                self.record.state().lock().await.set_ledger(ledger);
            }
            (
                JournalRecordKind::ModelAction { step },
                Frame::Action {
                    ledger,
                    calls,
                    complete,
                    ..
                },
            ) => {
                if self.pending.is_some()
                    || self.action.as_ref().is_some_and(|(_, old)| {
                        old.iter()
                            .any(|call| !self.known.contains_key(&call.intent()))
                    })
                {
                    return Err(JournalError::Corrupt.into());
                }
                let ledger = ledger.restore()?;
                if ledger.task().run() != self.record.binding() {
                    return Err(JournalError::Corrupt.into());
                }
                self.record.state().lock().await.set_ledger(ledger);
                self.unfinalized = false;
                self.known.clear();
                self.action = if complete { None } else { Some((*step, calls)) };
                self.status = if complete {
                    RunStatus::Completed
                } else {
                    RunStatus::Running
                };
            }
            (
                JournalRecordKind::Intent(expected),
                Frame::Intent {
                    intent,
                    call,
                    grant,
                },
            ) => {
                let intent = intent.restore()?;
                let call = call.restore()?;
                let grant = grant.restore()?;
                if intent != **expected
                    || self.pending.is_some()
                    || grant.authority() != intent.authority()
                    || intent.mutates() != call.writes()
                {
                    return Err(JournalError::Corrupt.into());
                }
                let planned = self
                    .action
                    .as_ref()
                    .and_then(|(_, calls)| {
                        calls.iter().find(|planned| planned.intent() == intent.id())
                    })
                    .ok_or(JournalError::Corrupt)?;
                let actual = planned.call()?;
                if actual.name() != call.name()
                    || actual.arguments().map_err(EngineError::from)?
                        != call.arguments().map_err(EngineError::from)?
                {
                    return Err(JournalError::Corrupt.into());
                }
                let mut grants = self.record.grants();
                grants.replace_tool(grant);
                self.record.set_grants(grants);
                self.pending = Some((intent, EffectStage::IntentCommitted));
                self.choice = None;
            }
            (
                JournalRecordKind::Dispatched { intent },
                Frame::Dispatch {
                    intent: original,
                    grant,
                },
            ) => {
                let pending = self.pending.as_mut().ok_or(JournalError::Corrupt)?;
                if *intent.as_bytes() != original
                    || pending.0.id() != *intent
                    || pending.1 != EffectStage::IntentCommitted
                {
                    return Err(JournalError::Corrupt.into());
                }
                let grant = grant.restore()?;
                if grant.authority() != pending.0.authority() {
                    return Err(JournalError::Corrupt.into());
                }
                let mut grants = self.record.grants();
                grants.replace_tool(grant);
                self.record.set_grants(grants);
                pending.1 = EffectStage::DispatchCommitted;
            }
            (
                JournalRecordKind::Outcome { intent, .. } | JournalRecordKind::Declined { intent },
                Frame::Result {
                    intent: original,
                    output,
                    failed,
                    unknown,
                    ..
                },
            ) => {
                let pending = self.pending.take().ok_or(JournalError::Corrupt)?;
                if *intent.as_bytes() != original || pending.0.id() != *intent {
                    return Err(JournalError::Corrupt.into());
                }
                let (step, calls) = self.action.as_ref().ok_or(JournalError::Corrupt)?;
                let call = calls
                    .iter()
                    .find(|call| call.intent() == *intent)
                    .ok_or(JournalError::Corrupt)?
                    .restore(*step)?;
                self.known.insert(
                    *intent,
                    EngineToolResult::new(call.slot(), BoundedJson::new(output)?, failed),
                );
                self.unknown |= unknown && pending.0.mutates();
                self.choice = None;
            }
            (JournalRecordKind::RunStatus, Frame::Status(tag)) => self.status = status(tag)?,
            (JournalRecordKind::Progress { .. }, Frame::Progress(progress)) => {
                progress.restore()?;
            }
            (JournalRecordKind::Approval { .. }, Frame::Approval { .. }) => {}
            (JournalRecordKind::Reconciled { intent, .. }, Frame::Reconciled { original, .. }) => {
                if original.restore()?.id() != *intent {
                    return Err(JournalError::Corrupt.into());
                }
                self.unknown = false;
                self.declared = true;
                self.status = RunStatus::Interrupted;
            }
            _ => return Err(JournalError::Corrupt.into()),
        }
        Ok(())
    }
    fn command(&mut self, command: &ReceivedCommand) -> Result<(), CoordinatorError> {
        let mut grants = self.record.grants();
        match command.body() {
            CommandBody::InferenceGrant(grant) => grants.install_inference(grant.clone())?,
            CommandBody::ToolGrant(grant) => grants.install_tool(grant.clone())?,
            CommandBody::RevokeGrant(reference) => {
                grants.revoke(*reference);
                self.status = RunStatus::Stopped;
            }
            CommandBody::RevokeIssuer(revoke) => {
                grants.revoke_issuer(revoke);
                self.status = RunStatus::Stopped;
            }
            CommandBody::Stop | CommandBody::DeleteChat => self.status = RunStatus::Stopped,
            CommandBody::Approval(answer) => {
                if !matches!(answer.choice(), ApprovalChoice::ReconcileKnown(_)) {
                    let (intent, stage) = self.pending.as_ref().ok_or(JournalError::Corrupt)?;
                    if *stage != EffectStage::IntentCommitted
                        || intent.id() != answer.intent()
                        || intent.authority() != answer.authority()
                        || intent.arguments() != answer.arguments()
                        || self.choice.is_some()
                    {
                        return Err(JournalError::Corrupt.into());
                    }
                    self.choice = Some(answer.choice());
                }
            }
            _ => {}
        }
        self.record.set_grants(grants);
        Ok(())
    }
    fn plan(&self, writer: WriterEpoch) -> Result<Option<RecoveryPlan>, CoordinatorError> {
        if self.unknown
            || self.declared
            || self.unfinalized
            || matches!(self.status, RunStatus::Completed | RunStatus::Stopped)
        {
            return Ok(None);
        }
        let Some((step, calls)) = &self.action else {
            return Ok(None);
        };
        let state = if let Some((intent, stage)) = &self.pending {
            if *stage != EffectStage::IntentCommitted {
                return Ok(None);
            }
            RunState::recover_undispatched(
                self.record.binding().clone(),
                self.record.context().destination().target().clone(),
                writer,
                self.revision,
                intent.clone(),
            )?
        } else {
            RunState::new(
                self.record.binding().clone(),
                self.record.context().destination().target().clone(),
                writer,
                self.revision,
            )
        };
        Ok(Some(RecoveryPlan {
            state,
            step: *step,
            calls: calls.clone(),
            known: self.known.clone(),
            pending: self.pending.as_ref().map(|(intent, _)| intent.clone()),
            choice: self.choice,
        }))
    }
}
fn status(tag: u8) -> Result<RunStatus, CoordinatorError> {
    Ok(match tag {
        0 => RunStatus::Queued,
        1 => RunStatus::Running,
        2 => RunStatus::WaitingApproval,
        3 => RunStatus::NeedsGrant,
        4 => RunStatus::NeedsUnlock,
        5 => RunStatus::NeedsProviderAccess,
        6 => RunStatus::OutcomeUnknown,
        7 => RunStatus::Interrupted,
        8 => RunStatus::Stopped,
        9 => RunStatus::Completed,
        _ => return Err(JournalError::Corrupt.into()),
    })
}

async fn load(
    services: &CoordinatorServices,
    chat: &Chat,
) -> Result<BTreeMap<RunBinding, ReplayedRun>, CoordinatorError> {
    if chat.log().position().records().get() == 0 {
        return Ok(BTreeMap::new());
    }
    let mut runs = BTreeMap::<RunBinding, ReplayedRun>::new();
    let replay =
        super::replay::JournalReplay::load(services, chat.log().lease().stream().clone()).await?;
    for stored in replay.records() {
        if matches!(
            stored.record().kind(),
            JournalRecordKind::BackgroundCommand { .. } | JournalRecordKind::PublicSnapshot
        ) {
            continue;
        }
        let frame = Frame::read(stored.record().payload())?;
        if let Frame::Accepted { command, allocated } = frame {
            let command = command.restore()?;
            if command.stream() != chat.log().lease().stream() {
                return Err(JournalError::Corrupt.into());
            }
            if let CommandBody::Submit(prompt) = command.body() {
                let run = allocated.ok_or(JournalError::Corrupt)?.restore()?;
                if stored.record().context().run() != Some(&run) || runs.contains_key(&run) {
                    return Err(JournalError::Corrupt.into());
                }
                runs.insert(
                    run.clone(),
                    ReplayedRun::new(Arc::new(RunRecord::new(run, prompt.clone())?)),
                );
            } else if let Some(run) = command.run() {
                runs.get_mut(run)
                    .ok_or(JournalError::Corrupt)?
                    .command(&command)?;
            } else if matches!(
                command.body(),
                CommandBody::RevokeIssuer(_) | CommandBody::DeleteChat
            ) {
                for run in runs.values_mut() {
                    run.command(&command)?;
                }
            }
        } else if matches!(
            stored.record().kind(),
            JournalRecordKind::QueueChanged { .. } | JournalRecordKind::Tombstone { .. }
        ) {
        } else {
            let run = stored
                .record()
                .context()
                .run()
                .ok_or(JournalError::Corrupt)?;
            runs.get_mut(run)
                .ok_or(JournalError::Corrupt)?
                .apply(stored, frame)
                .await?;
        }
    }
    Ok(runs)
}

pub(super) async fn resume(
    services: &CoordinatorServices,
    chat: &Chat,
    run: &RunRecord,
) -> Result<bool, CoordinatorError> {
    let replay = load(services, chat)
        .await?
        .remove(run.binding())
        .ok_or(JournalError::Corrupt)?;
    let plan = replay.plan(chat.log().lease().epoch())?;
    if replay.declared
        || replay.unknown
        || replay.unfinalized
        || matches!(
            replay.status,
            RunStatus::Completed
                | RunStatus::Stopped
                | RunStatus::Interrupted
                | RunStatus::OutcomeUnknown
        )
        || (plan.is_none() && (replay.action.is_some() || replay.pending.is_some()))
    {
        return Ok(false);
    }
    run.set_recovery(plan);
    run.set_grants(replay.record.grants());
    run.state()
        .lock()
        .await
        .set_ledger(replay.record.state().lock().await.ledger().clone());
    Ok(true)
}

pub(super) async fn restore(
    services: &CoordinatorServices,
    queue: &Arc<RunQueue>,
    chat: &Chat,
) -> Result<(), CoordinatorError> {
    let runs = load(services, chat).await?;
    for replay in runs.into_values() {
        let record = replay.record.clone();
        let now = services.timer().now();
        let dispatched = replay.pending.as_ref().is_some_and(|(intent, stage)| {
            *stage == EffectStage::DispatchCommitted && intent.mutates()
        });
        let plan = replay.plan(chat.log().lease().epoch())?;
        let resumable = !replay.declared
            && (plan.is_some()
                || (replay.action.is_none()
                    && replay.pending.is_none()
                    && matches!(
                        replay.status,
                        RunStatus::Queued
                            | RunStatus::NeedsGrant
                            | RunStatus::NeedsUnlock
                            | RunStatus::NeedsProviderAccess
                    )));
        let status = if replay.declared {
            RunStatus::Interrupted
        } else if replay.unknown || dispatched {
            RunStatus::OutcomeUnknown
        } else if replay.unfinalized {
            RunStatus::Interrupted
        } else {
            replay.status
        };
        record.state().lock().await.set_status(status);
        record.set_recovery(plan);
        services.register(&record)?;
        chat.insert(record.clone())?;
        if resumable
            && !dispatched
            && !replay.unknown
            && !replay.unfinalized
            && now < record.context().allowance().window().expires()
        {
            let held = queue.prepare(
                QueuedRun::new(
                    record.binding().clone(),
                    record.context().destination().target().clone(),
                    RunAccess::Mutating,
                    record.context().allowance().window().expires(),
                ),
                now,
            )?;
            held.admitted();
            if record.grants().inference().is_some() {
                queue.activate(record.binding(), now)?;
            }
        } else if !matches!(status, RunStatus::Completed) {
            record.control().fence();
        }
    }
    Ok(())
}
