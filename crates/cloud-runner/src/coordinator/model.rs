//! Durable actual-HTTP admission and bounded progress without an engine-held lock.
use super::{
    chat::Chat,
    frame::{Frame, Ledger, Progress},
    run::RunRecord,
    services::CoordinatorServices,
};
use futures::{FutureExt, future::BoxFuture};
use nocterm_cloud_model::{coordinator::*, engine::*, identity::*, inference::*, journal::*};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(super) struct ModelAdmission {
    services: Arc<CoordinatorServices>,
    chat: Arc<Chat>,
    run: Arc<RunRecord>,
    step: StepId,
    attempt: AttemptId,
    grant: InferenceGrant,
    pricing: Arc<dyn ModelPricing>,
    reserved: Arc<AtomicBool>,
}
impl ModelAdmission {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        chat: Arc<Chat>,
        run: Arc<RunRecord>,
        step: StepId,
        attempt: AttemptId,
        pricing: Arc<dyn ModelPricing>,
    ) -> Result<Self, CoordinatorError> {
        let grant = run
            .grants()
            .inference()
            .cloned()
            .ok_or(CoordinatorError::NeedsGrant)?;
        Ok(Self {
            services,
            chat,
            run,
            step,
            attempt,
            grant,
            pricing,
            reserved: Arc::new(AtomicBool::new(false)),
        })
    }
    fn current(&self, request: &ModelCallRequest) -> Result<(), CoordinatorError> {
        self.services.tasks().check_healthy()?;
        if !self.services.tasks().running() {
            return Err(CoordinatorError::Stopped);
        }
        self.run.control().check(self.services.source().as_ref())?;
        self.chat.check()?;
        self.chat.log().require_current()?;
        if request.run() != self.run.binding() || request.step() != self.step {
            return Err(CoordinatorError::Conflict);
        }
        let grants = self.run.grants();
        let current = grants.inference().ok_or(CoordinatorError::NeedsGrant)?;
        if current.id() != self.grant.id()
            || current.epoch() != self.grant.epoch()
            || current.origin() != self.grant.origin()
        {
            return Err(CoordinatorError::NeedsGrant);
        }
        self.run
            .control()
            .check_window(self.services.timer().now(), self.grant.allowance().window())
    }
    pub(super) fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub(super) fn was_reserved(&self) -> bool {
        self.reserved.load(Ordering::Acquire)
    }
    async fn reserve_call(&self, request: ModelCallRequest) -> Result<(), CoordinatorError> {
        self.current(&request)?;
        if self.reserved.swap(true, Ordering::AcqRel) {
            return Err(CoordinatorError::Conflict);
        }
        let quote = self.pricing.quote(&request)?;
        let mut state = self.run.state().lock().await;
        let mut ledger = state.ledger().clone();
        ledger.reserve(
            self.grant.clone(),
            self.attempt,
            request.clone(),
            quote,
            self.services.timer().now(),
        )?;
        let record = JournalRecord::new(
            RecordContext::Run(self.run.binding().clone()),
            JournalRecordKind::BudgetReserved {
                step: self.step,
                attempt: self.attempt,
            },
            Frame::Budget(Ledger::from(&ledger)).bytes()?,
        )?;
        self.chat
            .log()
            .append(
                None,
                vec![(
                    record,
                    Some(CoordinatorEvent::Progress {
                        run: self.run.binding().clone(),
                        event: EngineEvent::Usage(ledger.reported_usage()?),
                    }),
                )],
                None,
            )
            .await?;
        state.set_ledger(ledger);
        drop(state);
        self.current(&request)
    }
    async fn observe_call(
        &self,
        request: ModelCallRequest,
        observation: ModelCallObservation,
    ) -> Result<(), CoordinatorError> {
        self.current(&request)?;
        if !self.reserved.load(Ordering::Acquire) {
            return Err(CoordinatorError::Conflict);
        }
        let mut state = self.run.state().lock().await;
        let mut ledger = state.ledger().clone();
        ledger.observe(self.attempt, &request, observation)?;
        let record = JournalRecord::new(
            RecordContext::Run(self.run.binding().clone()),
            JournalRecordKind::BudgetReserved {
                step: self.step,
                attempt: self.attempt,
            },
            Frame::Budget(Ledger::from(&ledger)).bytes()?,
        )?;
        self.chat
            .log()
            .append(
                None,
                vec![(
                    record,
                    Some(CoordinatorEvent::Progress {
                        run: self.run.binding().clone(),
                        event: EngineEvent::Usage(ledger.reported_usage()?),
                    }),
                )],
                None,
            )
            .await?;
        state.set_ledger(ledger);
        drop(state);
        self.current(&request)
    }
}
// The public port returns static futures. The Arc owns this immutable original
// call scope; no reference or later run can retarget a suspended callback.
impl ModelCallAdmission for ModelAdmission {
    fn reserve(&self, request: ModelCallRequest) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        async move { this.reserve_call(request).await.map_err(admission_error) }.boxed()
    }
    fn reconcile(
        &self,
        request: ModelCallRequest,
        observation: ModelCallObservation,
    ) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        async move {
            this.observe_call(request, observation)
                .await
                .map_err(admission_error)
        }
        .boxed()
    }
    fn check_current(&self, request: &ModelCallRequest) -> Result<(), EngineError> {
        self.current(request).map_err(admission_error)
    }
}
fn admission_error(error: CoordinatorError) -> EngineError {
    EngineError::Admission(nocterm_core::SharedCause::new(error))
}

#[derive(Clone)]
pub(super) struct StepProgress {
    services: Arc<CoordinatorServices>,
    chat: Arc<Chat>,
    run: Arc<RunRecord>,
    step: StepId,
    text: Arc<Mutex<String>>,
}
impl StepProgress {
    pub(super) fn new(
        services: Arc<CoordinatorServices>,
        chat: Arc<Chat>,
        run: Arc<RunRecord>,
        step: StepId,
    ) -> Self {
        Self {
            services,
            chat,
            run,
            step,
            text: Arc::new(Mutex::new(String::new())),
        }
    }
    pub(super) fn text(&self) -> String {
        self.text.lock().clone()
    }
    async fn write(&self, step: StepId, event: EngineEvent) -> Result<(), CoordinatorError> {
        if step != self.step {
            return Err(CoordinatorError::Conflict);
        }
        self.run.control().check(self.services.source().as_ref())?;
        if let EngineEvent::Usage(usage) = event {
            // Reserve/observe already published the complete cumulative fact
            // atomically with its real Budget frame. The later SDK callback
            // validates that exact original step's report, without another fact.
            if self.run.state().lock().await.ledger().observed_usage(step) != Some(usage) {
                return Err(InferenceError::InvalidObservation.into());
            }
            return self.run.control().check(self.services.source().as_ref());
        }
        if let EngineEvent::Text(text) = &event
            && text.channel() == TextChannel::Assistant
            && self.text.lock().len().saturating_add(text.text().len())
                > nocterm_cloud_model::payload::MAX_LOGICAL_PAYLOAD_BYTES
        {
            return Err(EngineError::BudgetExceeded.into());
        }
        let record = JournalRecord::new(
            RecordContext::Run(self.run.binding().clone()),
            JournalRecordKind::Progress { step },
            Frame::Progress(Progress::from(&event)).bytes()?,
        )?;
        self.chat
            .log()
            .append(
                None,
                vec![(
                    record,
                    Some(CoordinatorEvent::Progress {
                        run: self.run.binding().clone(),
                        event: event.clone(),
                    }),
                )],
                None,
            )
            .await?;
        if let EngineEvent::Text(text) = event
            && text.channel() == TextChannel::Assistant
        {
            self.text.lock().push_str(text.text());
        }
        self.run.control().check(self.services.source().as_ref())
    }
}
impl EngineEventSink for StepProgress {
    fn publish(
        &self,
        step: StepId,
        event: EngineEvent,
    ) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        async move {
            this.write(step, event)
                .await
                .map_err(|error| EngineError::Progress(nocterm_core::SharedCause::new(error)))
        }
        .boxed()
    }
}
