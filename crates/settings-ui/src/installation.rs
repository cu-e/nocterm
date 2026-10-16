//! Settings presentation retains its actual store and native capability inputs.
use nocterm_store::SettingsHandle;

/// One settings UI installation reused by current and newly opened windows.
#[derive(Clone)]
pub struct Installed {
    settings: SettingsHandle,
    deps: crate::Deps,
}

impl Installed {
    pub fn new(settings: SettingsHandle, deps: crate::Deps) -> Self {
        Self { settings, deps }
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
    pub(crate) fn sandbox_probe(&self) -> std::sync::Arc<dyn nocterm_ai::sandbox::SandboxProbe> {
        self.deps.sandbox_probe()
    }
}
