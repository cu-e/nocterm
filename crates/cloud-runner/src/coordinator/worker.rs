//! One foreground engine owner. Every model/tool boundary is durably admitted.
use super::{
    chat::Chat,
    effects::EffectHost,
    frame::{Frame, Ledger, PlannedCall},
    history,
    model::{ModelAdmission, StepProgress},
    run::RunRecord,
    services::CoordinatorServices,
};
use futures::future::AbortHandle;
use nocterm_cloud_model::{
    coordinator::*, engine::*, identity::*, journal::*, run::RunState, tool::RunRetirement,
};
use std::sync::Arc;

pub(super) struct RunWorker {
    services: Arc<CoordinatorServices>,
    chat: Arc<Chat>,
    run: Arc<RunRecord>,
}
impl RunWorker {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        chat: Arc<Chat>,
        run: Arc<RunRecord>,
    ) -> Self {
        Self {
            services,
            chat,
            run,
        }
    }
    pub(super) async fn execute(&self) -> Result<(), CoordinatorError> {
        self.run.resumed();
        let result = self.advance().await;
        let (status, retirement) = match &result {
            Ok(()) => (RunStatus::Completed, RunRetirement::Completed),
            Err(error) => failure(error),
        };
        if matches!(
            status,
            RunStatus::NeedsGrant | RunStatus::NeedsUnlock | RunStatus::NeedsProviderAccess
        ) {
            self.run.control().suspend();
        } else if status != RunStatus::Completed {
            self.run.control().fence();
        }
        // A tombstone remains terminal; cleanup may not append ordinary status.
        let publication = if self.chat.check().is_ok() {
            self.run.status(self.chat.log(), status).await
        } else {
            Ok(())
        };
        // A failed/fenced status publication must not skip local retirement.
        let retired = self.run.control().retire(retirement).await;
        publication?;
        retired?;
        result
    }
    async fn advance(&self) -> Result<(), CoordinatorError> {
        self.run.control().check(self.services.source().as_ref())?;
        let recovery = self.run.take_recovery();
        let effect_state = recovery.as_ref().map_or_else(
            || {
                RunState::new(
                    self.run.binding().clone(),
                    self.run.context().destination().target().clone(),
                    self.chat.log().lease().epoch(),
                    Revision::default(),
                )
            },
            |plan| plan.state(),
        );
        let mut effects = EffectHost::new(
            self.services.clone(),
            self.chat.clone(),
            self.run.clone(),
            effect_state,
        );
        if let Some(plan) = recovery {
            for (id, call) in plan.missing()? {
                if let Some(intent) = plan.pending().filter(|intent| intent.id() == id) {
                    effects
                        .undispatched(intent.clone(), call, plan.choice())
                        .await?;
                } else {
                    effects.execute(id, call).await?;
                }
            }
        }
        let configured = self
            .services
            .providers()
            .model(self.run.context().selection())?;
        let history = history::load(&self.services, self.run.binding()).await?;
        let mut prior = history.messages().to_vec();
        let input = match prior.last() {
            Some(EngineHistoryMessage::User(_)) => {
                let Some(EngineHistoryMessage::User(prompt)) = prior.pop() else {
                    return Err(JournalError::Corrupt.into());
                };
                EngineRequest::Prompt(prompt)
            }
            Some(EngineHistoryMessage::ToolResult(_)) => EngineRequest::ContinueHistory,
            _ => return Err(JournalError::Corrupt.into()),
        };
        let mut engine = configured.factory().create(
            EngineConfig {
                run: self.run.binding().clone(),
                model: configured.selection().model().clone(),
                history: EngineHistory::new(prior)?,
                budget: self.run.context().allowance().budget(),
            },
            None,
        )?;
        let mut input = input;
        self.run.status(self.chat.log(), RunStatus::Running).await?;
        loop {
            self.run.control().check(self.services.source().as_ref())?;
            let step = StepId::from_bytes(self.services.identity().fresh()?);
            let attempt = AttemptId::from_bytes(self.services.identity().fresh()?);
            let admission = ModelAdmission::new(
                self.services.clone(),
                self.chat.clone(),
                self.run.clone(),
                step,
                attempt,
                configured.pricing().clone(),
            )?;
            let progress = StepProgress::new(
                self.services.clone(),
                self.chat.clone(),
                self.run.clone(),
                step,
            );
            let (abort, cancellation) = AbortHandle::new_pair();
            self.run.control().install_abort(abort);
            let action = engine
                .advance(
                    EngineInput {
                        next_step: step,
                        request: input,
                    },
                    Arc::new(progress.clone()),
                    Arc::new(admission.clone()),
                    cancellation,
                )
                .await?;
            self.run.control().check(self.services.source().as_ref())?;
            if !admission.was_reserved() {
                // Rig can finish after accepting the previous turn's tool
                // results, before opening another HTTP call. Only the durable
                // hard turn ceiling permits that local completion. It creates
                // no model action, call reservation, usage or refund.
                let exhausted = matches!(&action, EngineAction::Complete(done)
                    if done.stop == nocterm_ai::model::session::StopReason::MaxTurnRequests);
                let state = self.run.state().lock().await;
                if exhausted
                    && state.ledger().charge()?.turns()
                        == self.run.context().allowance().budget().turns()
                {
                    return Ok(());
                }
                return Err(nocterm_cloud_model::inference::InferenceError::Conflict.into());
            }
            let calls = match &action {
                EngineAction::Tools(batch) => batch
                    .calls()
                    .iter()
                    .map(|call| {
                        Ok::<_, CoordinatorError>((
                            IntentId::from_bytes(self.services.identity().fresh()?),
                            call.clone(),
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                EngineAction::Complete(_) => Vec::new(),
            };
            let mut state = self.run.state().lock().await;
            let mut ledger = state.ledger().clone();
            ledger.finalize(admission.attempt())?;
            let record = JournalRecord::new(
                RecordContext::Run(self.run.binding().clone()),
                JournalRecordKind::ModelAction { step },
                Frame::Action {
                    ledger: Ledger::from(&ledger),
                    text: progress.text(),
                    calls: calls
                        .iter()
                        .map(|(intent, call)| PlannedCall::new(*intent, call))
                        .collect::<Result<_, _>>()?,
                    complete: matches!(action, EngineAction::Complete(_)),
                }
                .bytes()?,
            )?;
            // ModelAction is already visible as durable progress; this event
            // supplies its verified completion status, never partial authority.
            let status = if matches!(action, EngineAction::Complete(_)) {
                RunStatus::Completed
            } else {
                RunStatus::Running
            };
            self.chat
                .log()
                .model_action(
                    record,
                    CoordinatorEvent::Status {
                        run: self.run.binding().clone(),
                        status,
                    },
                    engine.as_mut(),
                )
                .await?;
            state.set_ledger(ledger);
            drop(state);
            self.run.control().check(self.services.source().as_ref())?;
            match action {
                EngineAction::Complete(_) => return Ok(()),
                EngineAction::Tools(batch) => {
                    let mut results = Vec::new();
                    for (id, call) in calls {
                        results.push(effects.execute(id, call).await?);
                    }
                    input =
                        EngineRequest::ToolResults(EngineToolResults::new(batch.step(), results)?);
                }
            }
        }
    }
}

fn failure(error: &CoordinatorError) -> (RunStatus, RunRetirement) {
    use nocterm_cloud_model::{inference::InferenceError, tool::ToolExecutionError};
    let status = match error {
        CoordinatorError::Engine(EngineError::Admission(cause) | EngineError::Progress(cause)) => {
            if let Some(original) = cause.downcast_ref::<CoordinatorError>() {
                return failure(original);
            }
            RunStatus::Interrupted
        }
        CoordinatorError::NeedsGrant
        | CoordinatorError::Inference(InferenceError::BudgetExceeded) => RunStatus::NeedsGrant,
        CoordinatorError::NeedsUnlock => RunStatus::NeedsUnlock,
        CoordinatorError::Tool(error) if error.is_needs_unlock() => RunStatus::NeedsUnlock,
        CoordinatorError::Engine(EngineError::NeedsProviderAccess)
        | CoordinatorError::NeedsProviderAccess => RunStatus::NeedsProviderAccess,
        CoordinatorError::Stopped | CoordinatorError::Engine(EngineError::Interrupted) => {
            RunStatus::Stopped
        }
        CoordinatorError::Tool(ToolExecutionError::OutcomeUnknown) => RunStatus::OutcomeUnknown,
        _ => RunStatus::Interrupted,
    };
    let retirement = if status == RunStatus::Stopped {
        RunRetirement::Stopped
    } else {
        RunRetirement::Interrupted
    };
    (status, retirement)
}
