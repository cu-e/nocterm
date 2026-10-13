//! Explicit native capability inputs retained by every settings window.
use nocterm_ai::sandbox::SandboxProbe;
use std::sync::Arc;
#[derive(Clone)]
pub struct Deps {
    sandbox_probe: Arc<dyn SandboxProbe>,
}
impl Deps {
    pub fn new(sandbox_probe: Arc<dyn SandboxProbe>) -> Self {
        Self { sandbox_probe }
    }
    pub(crate) fn sandbox_probe(&self) -> Arc<dyn SandboxProbe> {
        self.sandbox_probe.clone()
    }
}
