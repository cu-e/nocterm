//! Original bounded command context used by environment operations.
use crate::{ProjectSetupError, SetupFuture, SetupPhase, SetupProgress};
use nocterm_session::{ExecRequest, HostExec, local_fs::CancelFlag};
use std::sync::Arc;

pub trait EnvironmentCommands: Send + Sync + 'static {
    fn run(&self, phase: SetupPhase, request: ExecRequest) -> SetupFuture<String>;
    fn check(&self) -> Result<(), ProjectSetupError>;
    fn executor(&self) -> Arc<dyn HostExec>;
    fn with_executor(&self, executor: Arc<dyn HostExec>) -> Arc<dyn EnvironmentCommands>;
    fn observation(&self, executor: Option<Arc<dyn HostExec>>) -> Arc<dyn EnvironmentCommands>;
}
pub trait EnvironmentCommandFactory: Send + Sync + 'static {
    fn create(
        &self,
        executor: Arc<dyn HostExec>,
        cancel: CancelFlag,
        progress: SetupProgress,
    ) -> Arc<dyn EnvironmentCommands>;
}
