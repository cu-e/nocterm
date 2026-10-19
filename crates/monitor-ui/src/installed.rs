//! Explicit policy and optional local host retained for Monitor views and factories.
use gpui_kit::App;
use nocterm_session::HostExec;
use nocterm_store::SettingsHandle;
use std::sync::Arc;

#[derive(Clone)]
pub struct Installed {
    settings: SettingsHandle,
    local: Option<Arc<dyn HostExec>>,
}
impl Installed {
    pub(crate) fn settings(&self) -> &SettingsHandle {
        &self.settings
    }
    pub(crate) fn local(&self) -> Option<Arc<dyn HostExec>> {
        self.local.clone()
    }
}
pub fn install(
    local: Option<Arc<dyn HostExec>>,
    ready: &nocterm_ui::UiReady,
    cx: &mut App,
) -> Installed {
    ready.settings().register::<crate::MonitorSettings>(cx);
    Installed {
        settings: ready.settings().clone(),
        local,
    }
}
