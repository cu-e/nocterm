use super::{AdmissionState, EnvironmentOperation, EnvironmentOwner, OperationState};
use gpui::BackgroundExecutor;
use nocterm_project_providers::{EnvironmentError, EnvironmentSource, WorkspaceEnvironment};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct EnvironmentAdmission {
    state: Arc<Mutex<AdmissionState>>,
}
impl EnvironmentAdmission {
    pub(crate) fn admit(
        &self,
        port: Arc<dyn WorkspaceEnvironment>,
        source: EnvironmentSource,
        executor: BackgroundExecutor,
    ) -> Result<EnvironmentOperation, EnvironmentError> {
        let mut state = self.state.lock().map_err(|_| EnvironmentError::Canceled)?;
        if state.closing {
            return Err(EnvironmentError::Unavailable);
        }
        state
            .operations
            .retain(|operation| operation.strong_count() > 0);
        if state.operations.len() >= 32 {
            return Err(EnvironmentError::Busy);
        }
        let operation = EnvironmentOperation(Arc::new(EnvironmentOwner {
            port,
            source,
            executor,
            cancel: Default::default(),
            state: Mutex::new(OperationState::default()),
            admission: self.state.clone(),
            changed: event_listener::Event::new(),
        }));
        state.operations.push(Arc::downgrade(&operation.0));
        Ok(operation)
    }
    pub(crate) fn capture(
        &self,
    ) -> futures::future::BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
        let operations = self
            .state
            .lock()
            .map(|mut state| {
                state.closing = true;
                state
                    .operations
                    .drain(..)
                    .filter_map(|operation| operation.upgrade().map(EnvironmentOperation))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for operation in &operations {
            operation.cancel();
        }
        Box::pin(async move {
            futures::future::join_all(operations.iter().map(EnvironmentOperation::drain)).await;
            Ok(())
        })
    }
}
