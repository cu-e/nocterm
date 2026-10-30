//! Actual daemon execution composition. Configuration never carries API secrets.
mod budgets;
#[cfg(target_os = "linux")]
mod compose;
pub(crate) mod config;
#[cfg(target_os = "linux")]
mod runtime;
use crate::{RunnerError, sessions::config::SessionTargetConfiguration};
use budgets::ExecutionBudgets;
use config::ExecutionConfig;
use nocterm_cloud_model::journal::PublicationWorkBudget;
#[cfg(target_os = "linux")]
pub(crate) use runtime::ExecutionRuntime;
use std::sync::Arc;

/// Complete explicit configuration and its one shared bounded pool generation.
/// The daemon consumes this once; health/setup commands never start execution.
pub(crate) struct ExecutionSetup {
    configuration: ExecutionConfig,
    targets: SessionTargetConfiguration,
    budgets: ExecutionBudgets,
}
impl ExecutionSetup {
    pub(crate) fn capture(
        configuration: Option<ExecutionConfig>,
        targets: Option<SessionTargetConfiguration>,
        cleanup_bytes: usize,
    ) -> Result<Option<Self>, RunnerError> {
        match (configuration, targets) {
            (None, None) => Ok(None),
            (Some(configuration), Some(targets)) => {
                configuration.maximum_grants()?;
                targets.default_destination()?;
                Ok(Some(Self {
                    configuration,
                    targets,
                    budgets: ExecutionBudgets::new(cleanup_bytes)?,
                }))
            }
            _ => Err(RunnerError::Value("execution/session_targets")),
        }
    }
    pub(crate) fn transport_work(&self) -> Arc<dyn PublicationWorkBudget> {
        self.budgets.transport()
    }
}
