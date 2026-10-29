use super::{InferenceError, InferenceGrant, InferenceTask, ModelQuote};
use crate::{
    engine::{ModelCallObservation, ModelCallRequest, UsageObservation},
    identity::{AttemptId, StepId, Timestamp},
};
use std::collections::BTreeSet;

/// Descriptive original reservation. Restoring it never releases HTTP admission.
#[derive(Clone, Debug)]
pub struct InferenceReservation {
    attempt: AttemptId,
    grant: InferenceGrant,
    request: ModelCallRequest,
    quote: ModelQuote,
    output_bytes: usize,
    admitted: Timestamp,
}
impl InferenceReservation {
    pub fn from_journal(
        attempt: AttemptId,
        grant: InferenceGrant,
        request: ModelCallRequest,
        quote: ModelQuote,
        output_bytes: usize,
        admitted: Timestamp,
    ) -> Result<Self, InferenceError> {
        let allowance = grant.allowance();
        if request.run() != allowance.run() || quote.selection() != allowance.selection() {
            return Err(InferenceError::Binding);
        }
        if admitted < allowance.window().issued()
            || admitted >= allowance.window().expires()
            || output_bytes == 0
            || output_bytes > allowance.budget().output_bytes()
            || request.request_bytes() > allowance.budget().input_bytes()
            || request.output_token_limit() > allowance.budget().output_tokens()
            || quote.spend_micros() > allowance.spend_micros()
        {
            return Err(InferenceError::InvalidSnapshot);
        }
        Ok(Self {
            attempt,
            grant,
            request,
            quote,
            output_bytes,
            admitted,
        })
    }
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub fn grant(&self) -> &InferenceGrant {
        &self.grant
    }
    pub fn request(&self) -> &ModelCallRequest {
        &self.request
    }
    pub fn quote(&self) -> &ModelQuote {
        &self.quote
    }
    pub fn output_bytes(&self) -> usize {
        self.output_bytes
    }
    pub fn admitted(&self) -> Timestamp {
        self.admitted
    }
    fn validate_observation(
        &self,
        observation: &ModelCallObservation,
    ) -> Result<(), InferenceError> {
        let usage = observation.usage();
        if observation.output_bytes() > self.output_bytes
            || usage
                .output_tokens
                .is_some_and(|used| used > self.request.output_token_limit())
            || usage
                .cost_micros
                .is_some_and(|cost| cost > self.quote.spend_micros())
        {
            return Err(InferenceError::InvalidObservation);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct InferenceCallSnapshot {
    reservation: InferenceReservation,
    observation: Option<ModelCallObservation>,
    finalized: bool,
}
impl InferenceCallSnapshot {
    pub fn new(
        reservation: InferenceReservation,
        observation: Option<ModelCallObservation>,
        finalized: bool,
    ) -> Result<Self, InferenceError> {
        if finalized && observation.is_none() {
            return Err(InferenceError::InvalidSnapshot);
        }
        if let Some(value) = &observation {
            reservation.validate_observation(value)?;
        }
        Ok(Self {
            reservation,
            observation,
            finalized,
        })
    }
    pub fn reservation(&self) -> &InferenceReservation {
        &self.reservation
    }
    pub fn observation(&self) -> Option<&ModelCallObservation> {
        self.observation.as_ref()
    }
    pub fn finalized(&self) -> bool {
        self.finalized
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InferenceCharge {
    calls: u32,
    turns: u32,
    input_bytes: usize,
    output_bytes: usize,
    output_tokens: u64,
    spend: u64,
}
impl InferenceCharge {
    pub fn calls(self) -> u32 {
        self.calls
    }
    pub fn turns(self) -> u32 {
        self.turns
    }
    pub fn input_bytes(self) -> usize {
        self.input_bytes
    }
    pub fn output_bytes(self) -> usize {
        self.output_bytes
    }
    pub fn output_tokens(self) -> u64 {
        self.output_tokens
    }
    /// Conservative committed allowance, not necessarily a provider invoice.
    pub fn spend_micros(self) -> u64 {
        self.spend
    }
    fn add(self, call: &InferenceCallSnapshot, new_step: bool) -> Result<Self, InferenceError> {
        let original = &call.reservation;
        let known = call.observation.as_ref().filter(|_| call.finalized);
        let output_bytes = known.map_or(original.output_bytes, ModelCallObservation::output_bytes);
        let output_tokens = known
            .and_then(|value| value.usage().output_tokens)
            .unwrap_or(original.request.output_token_limit());
        let spend = known
            .and_then(|value| value.usage().cost_micros)
            .unwrap_or(original.quote.spend_micros());
        Ok(Self {
            calls: self
                .calls
                .checked_add(1)
                .ok_or(InferenceError::BudgetExceeded)?,
            turns: self
                .turns
                .checked_add(u32::from(new_step))
                .ok_or(InferenceError::BudgetExceeded)?,
            input_bytes: self
                .input_bytes
                .checked_add(original.request.request_bytes())
                .ok_or(InferenceError::BudgetExceeded)?,
            output_bytes: self
                .output_bytes
                .checked_add(output_bytes)
                .ok_or(InferenceError::BudgetExceeded)?,
            output_tokens: self
                .output_tokens
                .checked_add(output_tokens)
                .ok_or(InferenceError::BudgetExceeded)?,
            spend: self
                .spend
                .checked_add(spend)
                .ok_or(InferenceError::BudgetExceeded)?,
        })
    }
    fn fits(self, task: &InferenceTask) -> bool {
        let budget = task.budget();
        self.calls <= budget.calls()
            && self.turns <= budget.turns()
            && self.input_bytes <= budget.input_bytes()
            && self.output_bytes <= budget.output_bytes()
            && self.output_tokens <= budget.output_tokens()
            && self.spend <= task.spend_micros()
    }
}

/// Checked recovery data, not a restored network/execution capability.
#[derive(Clone, Debug)]
pub struct InferenceSnapshot {
    task: InferenceTask,
    calls: Vec<InferenceCallSnapshot>,
    observed: Timestamp,
    clock_failed: bool,
}
impl InferenceSnapshot {
    pub fn new(
        task: InferenceTask,
        calls: Vec<InferenceCallSnapshot>,
        observed: Timestamp,
        clock_failed: bool,
    ) -> Result<Self, InferenceError> {
        if calls.len() > task.budget().calls() as usize || observed < task.window().issued() {
            return Err(InferenceError::InvalidSnapshot);
        }
        let mut attempts = BTreeSet::new();
        let mut steps = BTreeSet::<StepId>::new();
        let mut charge = InferenceCharge::default();
        let mut admitted = task.window().issued();
        for call in &calls {
            let original = &call.reservation;
            if !task.contains(original.grant.allowance())
                || original.admitted > observed
                || original.admitted < admitted
                || !attempts.insert(original.attempt)
                || !steps.insert(original.request.step())
            {
                return Err(InferenceError::InvalidSnapshot);
            }
            admitted = original.admitted;
            charge = charge.add(call, true)?;
        }
        if !charge.fits(&task) {
            return Err(InferenceError::InvalidSnapshot);
        }
        Ok(Self {
            task,
            calls,
            observed,
            clock_failed,
        })
    }
    pub fn task(&self) -> &InferenceTask {
        &self.task
    }
    pub fn calls(&self) -> &[InferenceCallSnapshot] {
        &self.calls
    }
    pub fn observed(&self) -> Timestamp {
        self.observed
    }
    pub fn clock_failed(&self) -> bool {
        self.clock_failed
    }
}

/// Cumulative run accounting survives grant replacement. Native owners mutate a
/// preview and publish it only after the corresponding journal transaction ACK.
/// Current issuer/grant/writer/source checks remain separate mandatory guards.
#[derive(Clone, Debug)]
pub struct InferenceLedger {
    snapshot: InferenceSnapshot,
}
impl InferenceLedger {
    pub fn new(task: InferenceTask) -> Self {
        let observed = task.window().issued();
        Self {
            snapshot: InferenceSnapshot {
                task,
                calls: Vec::new(),
                observed,
                clock_failed: false,
            },
        }
    }
    pub fn from_snapshot(snapshot: InferenceSnapshot) -> Self {
        Self { snapshot }
    }
    pub fn snapshot(&self) -> InferenceSnapshot {
        self.snapshot.clone()
    }
    pub fn task(&self) -> &InferenceTask {
        &self.snapshot.task
    }
    pub fn charge(&self) -> Result<InferenceCharge, InferenceError> {
        let mut steps = BTreeSet::new();
        self.snapshot
            .calls
            .iter()
            .try_fold(InferenceCharge::default(), |charge, call| {
                charge.add(call, steps.insert(call.reservation.request.step()))
            })
    }
    /// The original report for a step, if durably observed by the native owner.
    /// Reserve and snapshot validation forbid a second attempt for the same step;
    /// this lookup cannot select an arbitrary/latest call from several attempts.
    pub fn observed_usage(&self, step: StepId) -> Option<UsageObservation> {
        self.snapshot
            .calls
            .iter()
            .find(|call| call.reservation.request.step() == step)
            .and_then(|call| call.observation.as_ref())
            .map(ModelCallObservation::usage)
    }
    /// Complete optional cumulative provider reports for this run. Each missing
    /// call observation or field makes only that sum unknown. Reservations and
    /// conservative charges never substitute for reported tokens or money.
    /// An empty ledger is unknown rather than an invented known zero.
    pub fn reported_usage(&self) -> Result<UsageObservation, InferenceError> {
        fn sum(
            calls: &[InferenceCallSnapshot],
            field: fn(UsageObservation) -> Option<u64>,
        ) -> Result<Option<u64>, InferenceError> {
            if calls.is_empty() {
                return Ok(None);
            }
            calls.iter().try_fold(Some(0u64), |total, call| {
                match (
                    total,
                    call.observation.as_ref().and_then(|o| field(o.usage())),
                ) {
                    (Some(total), Some(value)) => total
                        .checked_add(value)
                        .map(Some)
                        .ok_or(InferenceError::UsageOverflow),
                    _ => Ok(None),
                }
            })
        }
        let calls = &self.snapshot.calls;
        Ok(UsageObservation {
            input_tokens: sum(calls, |u| u.input_tokens)?,
            output_tokens: sum(calls, |u| u.output_tokens)?,
            total_tokens: sum(calls, |u| u.total_tokens)?,
            cached_input_tokens: sum(calls, |u| u.cached_input_tokens)?,
            thought_tokens: sum(calls, |u| u.thought_tokens)?,
            cost_micros: sum(calls, |u| u.cost_micros)?,
        })
    }
    pub fn check(&mut self, grant: &InferenceGrant, now: Timestamp) -> Result<(), InferenceError> {
        if !self.task().contains(grant.allowance()) {
            return Err(InferenceError::Binding);
        }
        if self.snapshot.clock_failed || now < self.snapshot.observed {
            self.snapshot.clock_failed = true;
            return Err(InferenceError::ClockRollback);
        }
        self.snapshot.observed = now;
        if now < grant.allowance().window().issued()
            || now >= grant.allowance().window().expires()
            || now >= self.task().window().expires()
        {
            return Err(InferenceError::Expired);
        }
        Ok(())
    }
    /// One original step/attempt may reserve once. A restored reservation is
    /// never a reason to repeat HTTP with the same or a renamed attempt ID.
    pub fn reserve(
        &mut self,
        grant: InferenceGrant,
        attempt: AttemptId,
        request: ModelCallRequest,
        quote: ModelQuote,
        now: Timestamp,
    ) -> Result<InferenceReservation, InferenceError> {
        self.check(&grant, now)?;
        if self.snapshot.calls.iter().any(|call| {
            call.reservation.attempt == attempt || call.reservation.request.step() == request.step()
        }) {
            return Err(InferenceError::Conflict);
        }
        if request.run() != self.task().run() || quote.selection() != self.task().selection() {
            return Err(InferenceError::Binding);
        }
        let before = self.charge()?;
        let allowance = grant.allowance();
        if request.request_bytes() > allowance.budget().input_bytes()
            || request.output_token_limit() > allowance.budget().output_tokens()
            || quote.spend_micros() > allowance.spend_micros()
        {
            return Err(InferenceError::BudgetExceeded);
        }
        let output_bytes = allowance
            .budget()
            .output_bytes()
            .checked_sub(before.output_bytes)
            .filter(|bytes| *bytes > 0)
            .ok_or(InferenceError::BudgetExceeded)?;
        let reservation =
            InferenceReservation::from_journal(attempt, grant, request, quote, output_bytes, now)?;
        let call = InferenceCallSnapshot::new(reservation.clone(), None, false)?;
        let after = before.add(&call, true)?;
        if !after.fits(reservation.grant.allowance()) || !after.fits(self.task()) {
            return Err(InferenceError::BudgetExceeded);
        }
        self.snapshot.calls.push(call);
        Ok(reservation)
    }
    /// Persists original verified observations separately from settlement. Even
    /// known observations release no allowance until successful model-action ACK.
    pub fn observe(
        &mut self,
        attempt: AttemptId,
        request: &ModelCallRequest,
        observation: ModelCallObservation,
    ) -> Result<(), InferenceError> {
        let call = self
            .snapshot
            .calls
            .iter_mut()
            .find(|call| call.reservation.attempt == attempt)
            .ok_or(InferenceError::Conflict)?;
        if &call.reservation.request != request {
            return Err(InferenceError::Conflict);
        }
        call.reservation.validate_observation(&observation)?;
        if let Some(original) = &call.observation {
            if original != &observation {
                return Err(InferenceError::Conflict);
            }
            return Ok(());
        }
        call.observation = Some(observation);
        Ok(())
    }
    /// Native code includes this preview in the same successful model action /
    /// checkpoint commit; a callback ACK alone must never call this transition.
    pub fn finalize(&mut self, attempt: AttemptId) -> Result<(), InferenceError> {
        let call = self
            .snapshot
            .calls
            .iter_mut()
            .find(|call| call.reservation.attempt == attempt)
            .ok_or(InferenceError::Conflict)?;
        if call.observation.is_none() {
            return Err(InferenceError::InvalidObservation);
        }
        call.finalized = true;
        Ok(())
    }
}
