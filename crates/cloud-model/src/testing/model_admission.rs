//! Scripted acknowledgements only: not fsync, crypto or production authorization.
use crate::{
    authority::RunBinding,
    engine::{EngineError, ModelCallAdmission, ModelCallObservation, ModelCallRequest},
};
use futures::future::BoxFuture;
use nocterm_session::testing::Gate;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

struct AdmissionState {
    run: RunBinding,
    requests: Vec<ModelCallRequest>,
    pause: Option<(Gate, Gate)>,
    failure: Option<EngineError>,
    revoked: bool,
    revoke_on_ack: bool,
    observations: Vec<(ModelCallRequest, ModelCallObservation)>,
    observe_pause: Option<(Gate, Gate)>,
    observe_failure: Option<EngineError>,
    revoke_on_observe: bool,
}

#[derive(Clone)]
pub struct ScriptedModelAdmission {
    state: Arc<Mutex<AdmissionState>>,
}
impl ScriptedModelAdmission {
    pub fn new(run: RunBinding) -> Self {
        Self {
            state: Arc::new(Mutex::new(AdmissionState {
                run,
                requests: Vec::new(),
                pause: None,
                failure: None,
                revoked: false,
                revoke_on_ack: false,
                observations: Vec::new(),
                observe_pause: None,
                observe_failure: None,
                revoke_on_observe: false,
            })),
        }
    }
    fn state(&self) -> MutexGuard<'_, AdmissionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// One reservation announces entry, then waits until release.
    pub fn pause_next(&self) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.state().pause = Some((entered.clone(), release.clone()));
        (entered, release)
    }
    pub fn fail_next(&self, error: EngineError) {
        self.state().failure = Some(error);
    }
    pub fn revoke(&self) {
        self.state().revoked = true;
    }
    /// Simulates an ACK immediately followed by host control invalidation.
    pub fn revoke_on_ack(&self) {
        self.state().revoke_on_ack = true;
    }
    pub fn pause_reconcile(&self) -> (Gate, Gate) {
        let entered = Gate::pending();
        let release = Gate::pending();
        self.state().observe_pause = Some((entered.clone(), release.clone()));
        (entered, release)
    }
    pub fn fail_reconcile(&self, error: EngineError) {
        self.state().observe_failure = Some(error);
    }
    pub fn revoke_on_reconcile_ack(&self) {
        self.state().revoke_on_observe = true;
    }
    pub fn observations(&self) -> Vec<(ModelCallRequest, ModelCallObservation)> {
        self.state().observations.clone()
    }
    pub fn requests(&self) -> Vec<ModelCallRequest> {
        self.state().requests.clone()
    }
}
impl ModelCallAdmission for ScriptedModelAdmission {
    fn reserve(&self, request: ModelCallRequest) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        Box::pin(async move {
            this.check_current(&request)?;
            let pause = {
                let mut state = this.state();
                if state.requests.len() >= 1024 {
                    return Err(EngineError::BudgetExceeded);
                }
                state.requests.push(request.clone());
                state.pause.take()
            };
            if let Some((entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            this.check_current(&request)?;
            let mut state = this.state();
            if let Some(error) = state.failure.take() {
                return Err(error);
            }
            if state.revoke_on_ack {
                state.revoked = true;
            }
            Ok(())
        })
    }
    fn reconcile(
        &self,
        request: ModelCallRequest,
        observation: ModelCallObservation,
    ) -> BoxFuture<'static, Result<(), EngineError>> {
        let this = self.clone();
        Box::pin(async move {
            this.check_current(&request)?;
            let pause = {
                let mut state = this.state();
                if !state.requests.contains(&request) {
                    return Err(EngineError::InvalidStream);
                }
                if let Some((_, original)) =
                    state.observations.iter().find(|(at, _)| at == &request)
                {
                    return if original == &observation {
                        Ok(())
                    } else {
                        Err(EngineError::InvalidStream)
                    };
                }
                state.observe_pause.take()
            };
            if let Some((entered, release)) = pause {
                entered.release();
                release.wait().await;
            }
            this.check_current(&request)?;
            let mut state = this.state();
            if let Some(error) = state.observe_failure.take() {
                return Err(error);
            }
            state.observations.push((request, observation));
            if state.revoke_on_observe {
                state.revoked = true;
            }
            Ok(())
        })
    }
    fn check_current(&self, request: &ModelCallRequest) -> Result<(), EngineError> {
        let state = self.state();
        if state.revoked {
            return Err(EngineError::Interrupted);
        }
        if &state.run != request.run() {
            return Err(EngineError::InvalidStream);
        }
        Ok(())
    }
}
