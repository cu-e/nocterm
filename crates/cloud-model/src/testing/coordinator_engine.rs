//! Shared deterministic engine driver; no provider network or durability proof.
use crate::{
    engine::*,
    identity::{NamespaceId, StreamRevision},
    payload::OpaqueBytes,
};
use futures::{
    FutureExt,
    future::{AbortRegistration, BoxFuture},
};
use nocterm_session::testing::Gate;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
};

/// One verified scripted provider turn. Byte/token limits are explicit fixtures,
/// not estimates used by the production coordinator.
#[derive(Clone)]
pub struct ScriptedEngineTurn {
    pub request_bytes: usize,
    pub output_cap: u64,
    pub events: Vec<EngineEvent>,
    pub observation: ModelCallObservation,
    pub action: EngineAction,
}
#[derive(Default)]
struct EngineScript {
    turns: VecDeque<ScriptedEngineTransition>,
    configs: Vec<EngineConfig>,
    inputs: Vec<EngineInput>,
    calls: usize,
    after_reserve: Option<(usize, Gate, Gate)>,
    after_reconcile: Option<(usize, Gate, Gate)>,
    failure_after_reserve: Option<(usize, EngineError)>,
}
enum ScriptedEngineTransition {
    Model(ScriptedEngineTurn),
    Local(EngineCompletion),
}
#[derive(Clone, Default)]
pub struct ScriptedEngineFactory {
    state: Arc<Mutex<EngineScript>>,
}
impl ScriptedEngineFactory {
    pub fn push(&self, turn: ScriptedEngineTurn) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .turns
            .push_back(ScriptedEngineTransition::Model(turn));
    }
    /// A completed SDK transition which makes no provider request or reservation.
    pub fn push_local_completion(&self, completion: EngineCompletion) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .turns
            .push_back(ScriptedEngineTransition::Local(completion));
    }
    pub fn pause_after_reserve(&self) -> (Gate, Gate) {
        self.pause_after_reserve_at(self.calls() + 1)
    }
    /// Pause the specified one-based actual model call after reserve ACK.
    pub fn pause_after_reserve_at(&self, call: usize) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .after_reserve = Some((call, entered.clone(), release.clone()));
        (entered, release)
    }
    /// Pause after the original observation ACK, before any SDK Usage callback.
    pub fn pause_after_reconcile_at(&self, call: usize) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .after_reconcile = Some((call, entered.clone(), release.clone()));
        (entered, release)
    }
    pub fn fail_after_reserve_at(&self, call: usize, error: EngineError) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failure_after_reserve = Some((call, error));
    }
    pub fn calls(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
    }
    pub fn configs(&self) -> Vec<EngineConfig> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .configs
            .clone()
    }
    pub fn inputs(&self) -> Vec<EngineInput> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .inputs
            .clone()
    }
}
impl AgentEngineFactory for ScriptedEngineFactory {
    fn create(
        &self,
        config: EngineConfig,
        restore: Option<EngineCheckpoint>,
    ) -> Result<Box<dyn AgentEngine>, EngineError> {
        if restore.is_some() {
            return Err(EngineError::UnsupportedFormat);
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .configs
            .push(config.clone());
        Ok(Box::new(ScriptedEngine {
            factory: self.clone(),
            config,
        }))
    }
}
struct ScriptedEngine {
    factory: ScriptedEngineFactory,
    config: EngineConfig,
}
impl AgentEngine for ScriptedEngine {
    fn advance<'a>(
        &'a mut self,
        input: EngineInput,
        progress: Arc<dyn EngineEventSink>,
        admission: Arc<dyn ModelCallAdmission>,
        _cancellation: AbortRegistration,
    ) -> BoxFuture<'a, Result<EngineAction, EngineError>> {
        async move {
            let transition = {
                let mut state = self
                    .factory
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                state.inputs.push(input.clone());
                state.turns.pop_front().ok_or(EngineError::Unavailable)?
            };
            let turn = match transition {
                ScriptedEngineTransition::Model(turn) => turn,
                ScriptedEngineTransition::Local(completion) => {
                    return Ok(EngineAction::Complete(completion));
                }
            };
            let request = ModelCallRequest::new(
                self.config.run.clone(),
                input.next_step,
                turn.request_bytes,
                turn.output_cap,
            )?;
            admission.reserve(request.clone()).await?;
            let (call, pause) = {
                let mut state = self
                    .factory
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let call = state.calls + 1;
                let pause = if state
                    .after_reserve
                    .as_ref()
                    .is_some_and(|(at, _, _)| *at == call)
                {
                    state.after_reserve.take()
                } else {
                    None
                };
                (call, pause)
            };
            if let Some((_, entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            admission.check_current(&request)?;
            self.factory
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .calls += 1;
            let failure = {
                let mut state = self
                    .factory
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if state
                    .failure_after_reserve
                    .as_ref()
                    .is_some_and(|(at, _)| *at == call)
                {
                    state.failure_after_reserve.take().map(|(_, error)| error)
                } else {
                    None
                }
            };
            if let Some(error) = failure {
                return Err(error);
            }
            for event in &turn.events {
                if !matches!(event, EngineEvent::Usage(_)) {
                    progress.publish(input.next_step, event.clone()).await?;
                }
            }
            admission
                .reconcile(request.clone(), turn.observation)
                .await?;
            let pause = {
                let mut state = self
                    .factory
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if state
                    .after_reconcile
                    .as_ref()
                    .is_some_and(|(at, _, _)| *at == call)
                {
                    state.after_reconcile.take()
                } else {
                    None
                }
            };
            if let Some((_, entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            admission.check_current(&request)?;
            // Match the actual Rig adapter: reported per-call usage follows
            // the original observation's durable acknowledgement.
            for event in turn.events {
                if matches!(event, EngineEvent::Usage(_)) {
                    progress.publish(input.next_step, event).await?;
                }
            }
            // Scripted calls are returned under the actual host-assigned step,
            // preserving each provider id and slot index in durable recovery.
            match turn.action {
                EngineAction::Tools(batch) => {
                    let calls = batch
                        .calls()
                        .iter()
                        .map(|call| {
                            EngineToolCall::new(
                                ToolSlot::new(input.next_step, call.slot().index())?,
                                call.provider_id().cloned(),
                                call.call().clone(),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(EngineAction::Tools(EngineToolBatch::new(
                        input.next_step,
                        calls,
                    )?))
                }
                action => Ok(action),
            }
        }
        .boxed()
    }
    fn checkpoint(
        &self,
        journal_revision: StreamRevision,
    ) -> Result<EngineCheckpoint, EngineError> {
        EngineCheckpoint::new(
            self.config.run.clone(),
            NamespaceId::new("scripted-engine-v1").map_err(|_| EngineError::UnsupportedFormat)?,
            journal_revision,
            OpaqueBytes::new(vec![1])?,
        )
    }
}
