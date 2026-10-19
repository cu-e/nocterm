//! A consumer-specific projection of the explicitly supplied published settings owner.
use gpui_kit::App;
use nocterm_connections_store::{CountryPolicySnapshot, CountryPolicySource};
use nocterm_store::SettingsHandle;
pub(crate) struct SettingsCountryPolicy {
    settings: SettingsHandle,
}
impl SettingsCountryPolicy {
    pub(crate) fn new(settings: SettingsHandle) -> Self {
        Self { settings }
    }
}
impl CountryPolicySource for SettingsCountryPolicy {
    fn current(&self, cx: &App) -> CountryPolicySnapshot {
        CountryPolicySnapshot::new(
            self.settings
                .setting::<nocterm_ui::AppearanceSettings>(cx)
                .detect_server_country,
            self.settings.revision(cx),
        )
    }
}
