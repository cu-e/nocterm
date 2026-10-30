//! Execution needs native containment, which exists only on Linux. A configured
//! non-Linux daemon is refused rather than served without it.
use super::ExecutionSetup;
use crate::RunnerError;
use std::sync::Arc;

pub(crate) enum ExecutionRuntime {}
impl ExecutionRuntime {
    pub(crate) async fn start(
        _: ExecutionSetup,
        _: &crate::bootstrap::State,
        _: nocterm_hub_io::cloud::RunnerHubPeer,
    ) -> Result<Self, RunnerError> {
        Err(RunnerError::Platform)
    }
    pub(crate) fn readiness(&self) -> Arc<std::sync::atomic::AtomicBool> {
        match *self {}
    }
    pub(crate) async fn stopped(&mut self) -> Result<(), RunnerError> {
        match *self {}
    }
    pub(crate) async fn drain(self, _: nocterm_core::persist::ShutdownDeadline) -> bool {
        match self {}
    }
}
