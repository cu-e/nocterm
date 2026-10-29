//! Immutable once scopes shared only by one engine and its own HTTP transport.
use futures::future::AbortHandle;
use nocterm_cloud_model::{
    authority::RunBinding,
    engine::{
        EngineError, EngineEventSink, ModelCallAdmission, ModelCallObservation, ModelCallRequest,
    },
    identity::StepId,
};
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct AdvanceContext {
    run: RunBinding,
    step: StepId,
    progress: Arc<dyn EngineEventSink>,
    admission: Arc<dyn ModelCallAdmission>,
    cancellation: AbortHandle,
}
impl AdvanceContext {
    pub(crate) fn new(
        run: RunBinding,
        step: StepId,
        progress: Arc<dyn EngineEventSink>,
        admission: Arc<dyn ModelCallAdmission>,
        cancellation: AbortHandle,
    ) -> Self {
        Self {
            run,
            step,
            progress,
            admission,
            cancellation,
        }
    }
    pub(crate) fn step(&self) -> StepId {
        self.step
    }
    pub(crate) fn progress(&self) -> &Arc<dyn EngineEventSink> {
        &self.progress
    }
}

#[derive(Clone)]
pub(crate) struct ModelCalls {
    run: RunBinding,
    current: Arc<Mutex<Option<Arc<CallScope>>>>,
}
impl ModelCalls {
    pub(crate) fn new(run: RunBinding) -> Self {
        Self {
            run,
            current: Arc::new(Mutex::new(None)),
        }
    }
    fn current(&self) -> MutexGuard<'_, Option<Arc<CallScope>>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(crate) fn begin(
        &self,
        context: &AdvanceContext,
        output_tokens: u64,
    ) -> Result<CallLease, EngineError> {
        let mut current = self.current();
        if context.run != self.run || current.is_some() {
            return Err(EngineError::InvalidStream);
        }
        let scope = Arc::new(CallScope {
            run: self.run.clone(),
            step: context.step,
            output_tokens,
            admission: context.admission.clone(),
            cancellation: context.cancellation.clone(),
            active: AtomicBool::new(true),
            sent: AtomicBool::new(false),
            failure: Mutex::new(None),
            request: Mutex::new(None),
            observation: Mutex::new(None),
        });
        *current = Some(scope.clone());
        Ok(CallLease {
            calls: self.clone(),
            scope,
        })
    }
    pub(crate) fn scope(&self) -> Result<Arc<CallScope>, EngineError> {
        self.current().clone().ok_or(EngineError::InvalidStream)
    }
}

pub(crate) struct CallScope {
    run: RunBinding,
    step: StepId,
    output_tokens: u64,
    admission: Arc<dyn ModelCallAdmission>,
    cancellation: AbortHandle,
    active: AtomicBool,
    sent: AtomicBool,
    failure: Mutex<Option<EngineError>>,
    request: Mutex<Option<ModelCallRequest>>,
    observation: Mutex<Option<(ModelCallObservation, bool)>>,
}
impl CallScope {
    fn failure(&self) -> MutexGuard<'_, Option<EngineError>> {
        self.failure.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn fail(&self, error: EngineError) -> EngineError {
        let error = self.failure().get_or_insert(error).clone();
        self.active.store(false, Ordering::Release);
        error
    }
    fn require_live(&self) -> Result<(), EngineError> {
        if !self.active.load(Ordering::Acquire) || self.cancellation.is_aborted() {
            return Err(self.fail(EngineError::Interrupted));
        }
        Ok(())
    }
    pub(crate) async fn reserve(&self, bytes: usize) -> Result<ModelCallRequest, EngineError> {
        self.require_live()?;
        if self
            .sent
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(self.fail(EngineError::InvalidStream));
        }
        let request = ModelCallRequest::new(self.run.clone(), self.step, bytes, self.output_tokens)
            .map_err(|error| self.fail(error))?;
        self.check(&request)?;
        self.admission
            .reserve(request.clone())
            .await
            .map_err(|error| self.fail(error))?;
        self.require_live()?;
        *self.request.lock().unwrap_or_else(PoisonError::into_inner) = Some(request.clone());
        Ok(request)
    }
    async fn reconcile(&self, observation: ModelCallObservation) -> Result<(), EngineError> {
        self.require_live()?;
        let request = self
            .request
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| self.fail(EngineError::InvalidStream))?;
        {
            let mut current = self
                .observation
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((original, acknowledged)) = current.as_ref() {
                return if *acknowledged && original == &observation {
                    self.check(&request)
                } else {
                    Err(self.fail(EngineError::InvalidStream))
                };
            }
            *current = Some((observation.clone(), false));
        }
        self.check(&request)?;
        self.admission
            .reconcile(request.clone(), observation.clone())
            .await
            .map_err(|error| self.fail(error))?;
        self.check(&request)?;
        *self
            .observation
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((observation, true));
        Ok(())
    }

    /// Called after reserve and provider lease checks, immediately before send.
    pub(crate) fn check(&self, request: &ModelCallRequest) -> Result<(), EngineError> {
        self.require_live()?;
        if request.run() != &self.run
            || request.step() != self.step
            || request.output_token_limit() != self.output_tokens
        {
            return Err(self.fail(EngineError::InvalidStream));
        }
        self.admission
            .check_current(request)
            .map_err(|error| self.fail(error))
    }
}

pub(crate) struct CallLease {
    calls: ModelCalls,
    scope: Arc<CallScope>,
}
impl CallLease {
    pub(crate) async fn reconcile(
        &self,
        observation: ModelCallObservation,
    ) -> Result<(), EngineError> {
        self.scope.reconcile(observation).await
    }
    pub(crate) fn check_current(&self) -> Result<(), EngineError> {
        let request = self
            .scope
            .request
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| self.scope.fail(EngineError::InvalidStream))?;
        self.scope.check(&request)
    }
    pub(crate) fn failure(&self) -> Option<EngineError> {
        self.scope.failure().clone()
    }
}
impl Drop for CallLease {
    fn drop(&mut self) {
        self.scope.active.store(false, Ordering::Release);
        let mut current = self.calls.current();
        if current
            .as_ref()
            .is_some_and(|scope| Arc::ptr_eq(scope, &self.scope))
        {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests;
