//! Shared bounded progress observations and acknowledgement gates for host tests.
use crate::{
    engine::{EngineError, EngineEvent, EngineEventSink},
    identity::StepId,
};
use futures::future::BoxFuture;
use nocterm_session::testing::Gate;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct EventState {
    events: Vec<(StepId, EngineEvent)>,
    pause: Option<(Gate, Gate)>,
    failure: Option<EngineError>,
}
#[derive(Clone, Default)]
pub struct ScriptedEngineEvents {
    state: Arc<Mutex<EventState>>,
}
impl ScriptedEngineEvents {
    fn state(&self) -> MutexGuard<'_, EventState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub fn events(&self) -> Vec<EngineEvent> {
        self.state()
            .events
            .iter()
            .map(|(_, event)| event.clone())
            .collect()
    }
    pub fn observations(&self) -> Vec<(StepId, EngineEvent)> {
        self.state().events.clone()
    }
    pub fn fail_next(&self, error: EngineError) {
        self.state().failure = Some(error);
    }
    pub fn pause_next(&self) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.state().pause = Some((entered.clone(), release.clone()));
        (entered, release)
    }
}
impl EngineEventSink for ScriptedEngineEvents {
    fn publish(
        &self,
        step: StepId,
        event: EngineEvent,
    ) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        Box::pin(async move {
            let pause = {
                let mut state = this.state();
                if state.events.len() >= 1024 {
                    return Err(EngineError::BudgetExceeded);
                }
                state.events.push((step, event));
                state.pause.take()
            };
            if let Some((entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            if let Some(error) = this.state().failure.take() {
                return Err(error);
            }
            Ok(())
        })
    }
}
