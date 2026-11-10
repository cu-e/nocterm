//! Inert shared fixture for independently scoped container filesystem tests.
use crate::{ProjectSetupError, WorkspaceCommandActivation};
use nocterm_session::ExecRequest;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct FakeWorkspaceCommandActivation {
    calls: Arc<Mutex<Vec<ExecRequest>>>,
}
impl FakeWorkspaceCommandActivation {
    pub fn calls(&self) -> Result<Vec<ExecRequest>, ProjectSetupError> {
        Ok(self
            .calls
            .lock()
            .map_err(|_| ProjectSetupError::Canceled)?
            .clone())
    }
}
impl WorkspaceCommandActivation for FakeWorkspaceCommandActivation {
    fn activate_request(&self, request: ExecRequest) -> ExecRequest {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.clone());
        request
    }
}
