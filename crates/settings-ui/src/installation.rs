//! A settings Item retains its actual document without native feature inputs.
#[derive(Clone)]
pub struct Installed {
    settings: nocterm_store::SettingsHandle,
}
impl Installed {
    pub fn new(settings: nocterm_store::SettingsHandle) -> Self {
        Self { settings }
    }
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
}
