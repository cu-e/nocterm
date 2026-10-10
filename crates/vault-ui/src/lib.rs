//! Vault settings and authentication views with explicit lifecycle dependencies.
//! Owns: settings page, unlock dialog and credential presentation adapter.
//! Does not own: encrypted format, persistence, workers or native authentication.
//! Ports used: VaultAccess. Ports provided: CredentialStore, SettingsPageSpec.
//! Settings: vault. Globals: none. Invariants: hidden/locked pages clear secrets.
mod credentials;
mod render;
mod settings;
mod unlock;
mod view;
pub use credentials::VaultCredentials;
use gpui_kit::{App, AppContext as _, Subscription};
use nocterm_ui::{SettingsExt as _, SettingsStore, UiReady};
use nocterm_vault::VaultAccess;
use nocterm_workspace::SettingsPageSpec;
pub use settings::VaultSettings;
use std::{sync::Arc, time::Duration};
pub use view::VaultView;
pub struct Deps {
    service: Arc<dyn VaultAccess>,
}
impl Deps {
    pub fn new(service: Arc<dyn VaultAccess>) -> Self {
        Self { service }
    }
}
pub struct Installed {
    service: Arc<dyn VaultAccess>,
    _settings: Subscription,
}
impl Installed {
    pub fn service(&self) -> Arc<dyn VaultAccess> {
        self.service.clone()
    }
}
pub fn install(deps: Deps, _ready: &UiReady, cx: &mut App) -> Installed {
    nocterm_ui::register_setting::<VaultSettings>(cx);
    let service = deps.service;
    unlock::init(service.clone(), cx);
    let settings_service = service.clone();
    let settings = cx.observe_global::<SettingsStore>(move |cx| {
        settings_service.set_auto_lock(Duration::from_secs(
            u64::from(cx.setting::<VaultSettings>().auto_lock_minutes) * 60,
        ));
    });
    Installed {
        service,
        _settings: settings,
    }
}
pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let service = installed.service();
    SettingsPageSpec::new("vault", "Vault", move |window, cx| {
        cx.new(|cx| VaultView::new(service.clone(), window, cx))
    })
    .with_icon(nocterm_ui::IconName::Lock)
}
#[cfg(test)]
mod tests;
