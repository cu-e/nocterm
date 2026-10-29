//! The runtime and signed observations share the public canonical argument hash.
use super::tools;
use nocterm_ai::TerminalCall;
use nocterm_cloud_model::{
    identity::Fingerprint,
    journal::{PublicationAllocation, PublicationWorkBudget},
    tool::{ArgumentFingerprint, ToolExecutionError},
};
use nocterm_core::SharedCause;
use std::sync::Arc;

pub struct RunnerArgumentFingerprint {
    work: Arc<dyn PublicationWorkBudget>,
}
impl RunnerArgumentFingerprint {
    pub fn new(work: Arc<dyn PublicationWorkBudget>) -> Self {
        Self { work }
    }
}
impl ArgumentFingerprint for RunnerArgumentFingerprint {
    fn fingerprint(&self, call: &TerminalCall) -> Result<Fingerprint, ToolExecutionError> {
        call.validate()?;
        // Only ExecCommand creates a vector of borrowed string views. The SDK
        // digest streams directly to its canonical hasher without a payload Vec.
        let bytes = match call {
            TerminalCall::ExecCommand(value) => value
                .args
                .len()
                .checked_mul(std::mem::size_of::<&str>())
                .ok_or(ToolExecutionError::Capacity)?,
            _ => 0,
        };
        let _allocation = self
            .work
            .try_reserve(
                PublicationAllocation::new(bytes.max(1), 1)
                    .map_err(|_| ToolExecutionError::Capacity)?,
            )
            .map_err(|_| ToolExecutionError::Capacity)?;
        let value = tools::arguments(call)
            .map_err(|error| ToolExecutionError::Execution(SharedCause::new(error)))?;
        value
            .digest()
            .map(Fingerprint::from_bytes)
            .map_err(|error| ToolExecutionError::Execution(SharedCause::new(error)))
    }
}
