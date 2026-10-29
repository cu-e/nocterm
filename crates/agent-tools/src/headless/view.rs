//! A detached run view never inherits a later run's admission.
use nocterm_cloud_model::tool::{ScopedToolRequest, ToolAuthority, ToolExecutionError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct ViewFence(AtomicBool);
impl ViewFence {
    pub(super) fn close(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(super) fn is_closed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(super) fn check(&self) -> Result<(), ToolExecutionError> {
        if self.is_closed() {
            Err(ToolExecutionError::StaleSession)
        } else {
            Ok(())
        }
    }
}
pub(super) struct ViewAuthority {
    fence: Arc<ViewFence>,
    original: Arc<dyn ToolAuthority>,
}
impl ViewAuthority {
    pub(super) fn new(fence: Arc<ViewFence>, original: Arc<dyn ToolAuthority>) -> Self {
        Self { fence, original }
    }
}
impl ToolAuthority for ViewAuthority {
    fn check(&self, request: &ScopedToolRequest) -> Result<(), ToolExecutionError> {
        self.fence.check()?;
        self.original.check(request)
    }
}
