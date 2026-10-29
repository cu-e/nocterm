//! New UI actions capture an account source; existing chats retain their old handle.
use crate::runtime::RuntimeHandle;
use nocterm_ai::{local_environment::LocalEnvironmentAccess, storage::AgentStorageError};
use nocterm_hubs_model::source_identity::LocalAccountSourceFactory;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct SourceBinding {
    runtime: RuntimeHandle,
    capture: Option<Arc<dyn LocalAccountSourceFactory>>,
}
impl SourceBinding {
    pub(super) fn fixed(runtime: RuntimeHandle) -> Self {
        Self {
            runtime,
            capture: None,
        }
    }
    pub(super) fn account(
        runtime: &RuntimeHandle,
        capture: Arc<dyn LocalAccountSourceFactory>,
    ) -> Self {
        // Explicitly unavailable, never a ThisComputer substitute before unlock.
        Self {
            runtime: runtime.unavailable_account(),
            capture: Some(capture),
        }
    }
    pub(super) fn runtime(&self) -> &RuntimeHandle {
        &self.runtime
    }
    pub(super) fn capture(&self) -> Result<RuntimeHandle, AgentStorageError> {
        if let Some(capture) = &self.capture {
            let access = LocalEnvironmentAccess::capture(capture.capture()?)?;
            self.runtime.for_environment(access)
        } else {
            self.runtime.check_current()?;
            Ok(self.runtime.clone())
        }
    }
}
