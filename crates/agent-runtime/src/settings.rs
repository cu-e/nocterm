//! The AI settings belong to the host; the runtime reads them through it.
use gpui::{App, Global};
use nocterm_ai::AiSettings;

/// How the runtime reads the AI settings as last published.
///
/// The host installs it before creating the runtime, and calls
/// [`Runtime::settings_changed`](super::Runtime::settings_changed) when they
/// change.
#[derive(Clone, Copy)]
pub struct AiSettingsSource(pub fn(&App) -> &AiSettings);

impl Global for AiSettingsSource {}

pub(crate) trait AiSettingsExt {
    fn ai(&self) -> &AiSettings;
    /// Whether AI features are switched on.
    fn ai_enabled(&self) -> bool;
}

impl AiSettingsExt for App {
    fn ai(&self) -> &AiSettings {
        (self.global::<AiSettingsSource>().0)(self)
    }
    fn ai_enabled(&self) -> bool {
        self.ai().enabled
    }
}
