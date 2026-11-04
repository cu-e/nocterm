//! Shared scripted captured source. No production verifier or encrypted store.
use super::{
    ConversationIntent, ConversationProjection, MAX_REPLAY_BYTES, ProjectionBatch,
    ProjectionBinding, ProjectionCursor, ProjectionError, RecordKind,
    ports::{
        AllocationRequirements, CapturedConversationSource, CommandDelivery, ConversationCommand,
        ConversationCoordinates, ConversationWork,
    },
};
use crate::identity::CommandId;
use futures::channel::oneshot;
use futures::{FutureExt as _, StreamExt as _, channel::mpsc, future::BoxFuture};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_session::testing::Gate;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

mod fixture;
pub use fixture::{
    ProjectionFixture, approval_events, execution_authority, prompt_event, run_binding,
};

#[derive(Clone, Default)]
pub struct ConversationEffects {
    pub staged: Vec<CommandId>,
    pub delivered: Vec<CommandId>,
    pub queried: Vec<CommandId>,
    pub cached: Vec<ProjectionCursor>,
    pub replayed: Vec<(ProjectionCursor, usize)>,
}
struct OperationControls {
    stage: Gate,
    delivery: Gate,
    cache: Gate,
    replay: Gate,
    lose_ack: bool,
    reject_next: bool,
    delivery_failure: Option<ProjectionError>,
    replay_failure: Option<ProjectionError>,
}
impl Default for OperationControls {
    fn default() -> Self {
        Self {
            stage: Gate::open(),
            delivery: Gate::open(),
            cache: Gate::open(),
            replay: Gate::open(),
            lose_ack: false,
            reject_next: false,
            delivery_failure: None,
            replay_failure: None,
        }
    }
}
struct SourceState {
    generation: u64,
    next_id: u128,
    staged: BTreeMap<CommandId, Arc<ConversationCommand>>,
    statuses: BTreeMap<CommandId, CommandDelivery>,
    effects: ConversationEffects,
    controls: OperationControls,
    ended: Gate,
    native_worker: Option<Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>>,
}
/// Captures an immutable membership generation. replace_membership ends every
/// old operation even when the next capture has the same account/binding IDs.
#[derive(Clone)]
pub struct ScriptedConversationSource {
    binding: ProjectionBinding,
    generation: u64,
    state: Arc<Mutex<SourceState>>,
    sender: Arc<Mutex<mpsc::Sender<ProjectionBatch>>>,
    receiver: Arc<futures::lock::Mutex<mpsc::Receiver<ProjectionBatch>>>,
    ended: Gate,
}
impl ScriptedConversationSource {
    pub fn new(binding: ProjectionBinding) -> Self {
        let (sender, receiver) = mpsc::channel(16);
        let ended = Gate::pending();
        Self {
            binding,
            generation: 1,
            state: Arc::new(Mutex::new(SourceState {
                generation: 1,
                next_id: 1,
                staged: BTreeMap::new(),
                statuses: BTreeMap::new(),
                effects: ConversationEffects::default(),
                controls: OperationControls::default(),
                ended: ended.clone(),
                native_worker: None,
            })),
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(futures::lock::Mutex::new(receiver)),
            ended,
        }
    }
    pub fn replace_membership(&self) -> Self {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ended.release();
        state.generation = state.generation.saturating_add(1);
        state.ended = Gate::pending();
        Self {
            binding: self.binding.clone(),
            generation: state.generation,
            state: self.state.clone(),
            sender: self.sender.clone(),
            receiver: self.receiver.clone(),
            ended: state.ended.clone(),
        }
    }
    /// Inject the real test host's detached native executor. Caller cancellation
    /// then drops only the oneshot waiter; worker/results retain charged values.
    pub fn run_native_workers_with(
        &self,
        worker: Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>,
    ) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .native_worker = Some(worker);
    }
    fn observe_native<T: Send + 'static>(
        &self,
        work: BoxFuture<'static, Result<T, ProjectionError>>,
    ) -> BoxFuture<'static, Result<T, ProjectionError>> {
        let worker = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .native_worker
            .clone();
        let Some(worker) = worker else {
            return work;
        };
        let (send, receive) = oneshot::channel();
        worker(
            async move {
                let _ = send.send(work.await);
            }
            .boxed(),
        );
        async move { receive.await.unwrap_or(Err(ProjectionError::Unavailable)) }.boxed()
    }
    pub fn queue(&self, page: ProjectionBatch) -> Result<(), ProjectionError> {
        self.sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_send(page)
            .map_err(|_| ProjectionError::Capacity)
    }
    pub fn effects(&self) -> ConversationEffects {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .clone()
    }
    pub fn staged(&self, id: CommandId) -> Option<Arc<ConversationCommand>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .staged
            .get(&id)
            .cloned()
    }
    pub fn lose_next_ack(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .lose_ack = true;
    }
    /// The shared semantic port supplies an already authenticated runner outcome,
    /// independently of transport storage. This fake performs no signature check.
    pub fn runner_receipt(
        &self,
        id: CommandId,
        outcome: CommandDelivery,
    ) -> Result<(), ProjectionError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.check(&state)?;
        if !state.staged.contains_key(&id) || outcome == CommandDelivery::Delivered {
            return Err(ProjectionError::InvalidRecord(RecordKind::Fixture));
        }
        state.statuses.insert(id, outcome);
        Ok(())
    }
    /// The next delivery fails with `error` before the hub stores anything, as an unreachable
    /// hub or runner does; the command stays staged.
    pub fn fail_next_delivery(&self, error: ProjectionError) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .delivery_failure = Some(error);
    }
    /// The next replay poll fails with `error`, as a poll that no runner answers does.
    pub fn fail_next_replay(&self, error: ProjectionError) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .replay_failure = Some(error);
    }
    pub fn reject_next_command(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .reject_next = true;
    }
    pub fn pause_stage(&self) -> Gate {
        let gate = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .stage = gate.clone();
        gate
    }
    pub fn pause_delivery(&self) -> Gate {
        let gate = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .delivery = gate.clone();
        gate
    }
    pub fn pause_cache(&self) -> Gate {
        let gate = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .cache = gate.clone();
        gate
    }
    pub fn pause_replay(&self) -> Gate {
        let gate = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .controls
            .replay = gate.clone();
        gate
    }
    fn check(&self, state: &SourceState) -> Result<(), ProjectionError> {
        (state.generation == self.generation)
            .then_some(())
            .ok_or(ProjectionError::Stale)
    }
}
impl CapturedConversationSource for ScriptedConversationSource {
    fn supports_prompt(&self) -> bool {
        true
    }
    fn allocation_requirements(
        &self,
        work: ConversationWork<'_>,
    ) -> Result<AllocationRequirements, ProjectionError> {
        self.check_current()?;
        Ok(match work {
            ConversationWork::Replay { maximum_bytes, .. } => {
                AllocationRequirements::new(maximum_bytes, 1)
            }
            // The fake backend stores submitted records as simulated durable
            // state; this bounded staging scratch represents native output work.
            ConversationWork::Stage { .. } | ConversationWork::Cache { .. } => {
                AllocationRequirements::new(4096, 1)
            }
        })
    }
    fn binding(&self) -> &ProjectionBinding {
        &self.binding
    }
    fn check_current(&self) -> Result<(), ProjectionError> {
        self.check(&self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        let ended = self.ended.clone();
        async move { ended.wait().await }.boxed()
    }
    fn with_current(&self, publish: &mut dyn FnMut()) -> Result<(), ProjectionError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.check(&state)?;
        publish();
        Ok(())
    }
    fn next(
        &self,
        after: ProjectionCursor,
        allocation: AllocationLease,
    ) -> BoxFuture<'static, Result<crate::projection::ports::ConversationReplay, ProjectionError>>
    {
        let maximum_bytes = allocation.bytes();
        let source = self.clone();
        self.observe_native(
            async move {
                source.check_current()?;
                let gate = {
                    let mut state = source.state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.effects.replayed.push((after, maximum_bytes));
                    if let Some(error) = state.controls.replay_failure.take() {
                        return Err(error);
                    }
                    state.controls.replay.clone()
                };
                let page = source
                    .receiver
                    .lock()
                    .await
                    .next()
                    .await
                    .ok_or(ProjectionError::Unavailable)?;
                gate.wait().await;
                source.check_current()?;
                if maximum_bytes > MAX_REPLAY_BYTES || page.resident_bytes() > maximum_bytes {
                    return Err(ProjectionError::Capacity);
                }
                Ok(crate::projection::ports::ConversationReplay::Page(
                    Box::new(Allocated::new(page, allocation)),
                ))
            }
            .boxed(),
        )
    }
    fn prepare(
        &self,
        coordinates: ConversationCoordinates,
        action: ConversationIntent,
    ) -> Result<ConversationCommand, ProjectionError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.check(&state)?;
        if coordinates.binding != self.binding {
            return Err(ProjectionError::WrongSource);
        }
        let id = CommandId::from_bytes(state.next_id.to_le_bytes());
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or(ProjectionError::Capacity)?;
        Ok(ConversationCommand {
            id,
            coordinates,
            action: action.clone(),
            preparation: crate::projection::ports::ConversationPreparation::new(
                crate::identity::Timestamp::from_millis(1),
                crate::identity::Timestamp::from_millis(match &action {
                    ConversationIntent::AnswerApproval(request) => {
                        10_000.min(request.expires().as_millis())
                    }
                    _ => 10_000,
                }),
                crate::identity::Fingerprint::from_bytes([1; 32]),
            )?,
        })
    }
    fn stage(
        &self,
        command: Arc<Allocated<ConversationCommand>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        let source = self.clone();
        self.observe_native(
            async move {
                source.check_current()?;
                let gate = source
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .controls
                    .stage
                    .clone();
                gate.wait().await;
                let mut state = source.state.lock().unwrap_or_else(PoisonError::into_inner);
                source.check(&state)?;
                if let Some(old) = state.staged.get(&command.value().id) {
                    return (old.coordinates == command.value().coordinates
                        && old.action == command.value().action
                        && old.preparation == command.value().preparation)
                        .then_some(())
                        .ok_or(ProjectionError::InvalidRecord(RecordKind::Fixture));
                }
                if state.staged.len() >= 1024 {
                    return Err(ProjectionError::Capacity);
                }
                state.effects.staged.push(command.value().id);
                // This map is the fake durable backend, like its in-memory repository
                // peers; workers retain the actual charged submitted generation.
                state
                    .staged
                    .insert(command.value().id, Arc::new(command.value().clone()));
                drop(work);
                Ok(())
            }
            .boxed(),
        )
    }
    fn deliver(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        let source = self.clone();
        async move {
            let _work = work;
            source.check_current()?;
            let gate = source
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .controls
                .delivery
                .clone();
            gate.wait().await;
            let mut state = source.state.lock().unwrap_or_else(PoisonError::into_inner);
            source.check(&state)?;
            if !state.staged.contains_key(&id) {
                return Err(ProjectionError::Persistence);
            }
            if let Some(error) = state.controls.delivery_failure.take() {
                return Err(error);
            }
            state.effects.delivered.push(id);
            let outcome = if std::mem::take(&mut state.controls.reject_next) {
                CommandDelivery::Rejected
            } else {
                CommandDelivery::Delivered
            };
            // Matching the hub ACK supplies no implicit runner admission. An
            // explicit rejection control represents a verified R refusal.
            if outcome == CommandDelivery::Rejected {
                state.statuses.insert(id, outcome);
            } else {
                state.statuses.entry(id).or_insert(CommandDelivery::Pending);
            }
            if std::mem::take(&mut state.controls.lose_ack) {
                return Err(ProjectionError::OutcomeUnknown);
            }
            Ok(outcome)
        }
        .boxed()
    }
    fn query(
        &self,
        id: CommandId,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<CommandDelivery, ProjectionError>> {
        let source = self.clone();
        async move {
            let _work = work;
            let mut state = source.state.lock().unwrap_or_else(PoisonError::into_inner);
            source.check(&state)?;
            state.effects.queried.push(id);
            Ok(state
                .statuses
                .get(&id)
                .copied()
                .unwrap_or(CommandDelivery::NotFound))
        }
        .boxed()
    }
    fn cache(
        &self,
        projection: Arc<Allocated<ConversationProjection>>,
        work: AllocationLease,
    ) -> BoxFuture<'static, Result<(), ProjectionError>> {
        let source = self.clone();
        self.observe_native(
            async move {
                source.check_current()?;
                let gate = source
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .controls
                    .cache
                    .clone();
                gate.wait().await;
                let mut state = source.state.lock().unwrap_or_else(PoisonError::into_inner);
                source.check(&state)?;
                if projection.value().binding() != &source.binding {
                    return Err(ProjectionError::WrongSource);
                }
                state.effects.cached.push(projection.value().cursor());
                drop(work);
                Ok(())
            }
            .boxed(),
        )
    }
}
