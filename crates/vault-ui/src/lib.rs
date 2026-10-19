//! Vault settings and authentication views with explicit lifecycle dependencies.
//! Unlock prompt facts and scan progress remain private to their original masked-control owner.
//! Owns: settings page, private masked form controls, unlock dialog and credential presentation.
//! Does not own: encrypted format, persistence, workers or native authentication.
//! Ports used: supplied VaultAccess and SettingsHandle. Ports provided: CredentialStore, SettingsPageSpec.
//! Settings: vault. Globals: none. Invariants: hidden/locked pages clear secrets.
mod credentials;
mod settings;
mod unlock;
mod view;
pub use credentials::VaultCredentials;
use gpui_kit::{App, AppContext as _, Subscription};
use nocterm_ui::SettingsPageSpec;
use nocterm_ui::UiReady;
use nocterm_vault::VaultAccess;
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
    settings: nocterm_store::SettingsHandle,
    service: Arc<dyn VaultAccess>,
    _settings: Subscription,
}
impl Installed {
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    pub fn service(&self) -> Arc<dyn VaultAccess> {
        self.service.clone()
    }
}
pub fn install(deps: Deps, ready: &UiReady, cx: &mut App) -> Installed {
    ready.settings().register::<VaultSettings>(cx);
    let service = deps.service;
    unlock::init(service.clone(), cx);
    let settings_service = service.clone();
    let policy = ready.settings().clone();
    let settings = ready.settings().observe(cx, move |cx| {
        settings_service.set_auto_lock(Duration::from_secs(
            u64::from(policy.setting::<VaultSettings>(cx).auto_lock_minutes) * 60,
        ));
    });
    Installed {
        settings: ready.settings().clone(),
        service,
        _settings: settings,
    }
}
pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let service = installed.service();
    let settings = installed.settings().clone();
    SettingsPageSpec::new("vault", "Vault", move |window, cx| {
        cx.new(|cx| VaultView::new(service.clone(), settings.clone(), window, cx))
    })
    .with_icon(nocterm_ui::IconName::Lock)
}
#[cfg(test)]
mod tests;
