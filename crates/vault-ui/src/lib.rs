//! The embedded Vault settings page. Sensitive operations run on VaultService's worker.
//!
//! The page has four tabs: the vault's state with unlock and lock, the saved
//! credentials, the master password and device unlock, and options.
mod credentials;
mod render;
mod settings;
mod unlock;
mod view;

use gpui_kit::{App, AppContext as _, Global};
use nocterm_ui::{SettingsExt as _, SettingsStore};
use nocterm_vault::{DeviceUnlockProvider, VaultStatus};
use nocterm_workspace::SettingsPageSpec;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub use credentials::VaultCredentials;
pub use nocterm_vault::VaultService;
pub use settings::VaultSettings;
pub use view::VaultView;

struct Service(Arc<VaultService>);
impl Global for Service {}

/// The vault's status, updated whenever the vault worker publishes a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VaultStatusGlobal(pub VaultStatus);
impl Global for VaultStatusGlobal {}

/// Mirrors the service's status into [`VaultStatusGlobal`] for the UI.
fn watch_status(service: Arc<VaultService>, cx: &mut App) {
    let mut seen = service.revision();
    cx.set_global(VaultStatusGlobal(service.status()));
    cx.spawn(async move |cx| {
        loop {
            seen = service.changed(seen).await;
            let status = VaultStatusGlobal(service.status());
            cx.update(|cx| {
                if *cx.global::<VaultStatusGlobal>() != status {
                    cx.set_global(status);
                }
            });
        }
    })
    .detach();
}

/// Installs the shared service and returns it for authentication integration.
pub fn init(path: PathBuf, cx: &mut App) -> Result<Arc<VaultService>, nocterm_vault::VaultError> {
    init_with_device_unlock(path, None, cx)
}

/// The composition root injects the native adapter; this feature remains platform independent.
pub fn init_with_device_unlock(
    path: PathBuf,
    device: Option<Arc<dyn DeviceUnlockProvider>>,
    cx: &mut App,
) -> Result<Arc<VaultService>, nocterm_vault::VaultError> {
    nocterm_ui::register_setting::<crate::VaultSettings>(cx);
    let service = Arc::new(VaultService::new_with_device_unlock(
        path,
        Duration::from_secs(u64::from(cx.setting::<crate::VaultSettings>().auto_lock_minutes) * 60),
        device,
    )?);
    cx.set_global(Service(service.clone()));
    watch_status(service.clone(), cx);
    unlock::init(cx);
    cx.observe_global::<SettingsStore>(|cx| {
        cx.global::<Service>().0.set_auto_lock(Duration::from_secs(
            u64::from(cx.setting::<crate::VaultSettings>().auto_lock_minutes) * 60,
        ));
    })
    .detach();
    Ok(service)
}

/// Supplies the lazy Vault page to the application Settings host.
pub fn settings_page() -> SettingsPageSpec {
    SettingsPageSpec::new("vault", "Vault", |window, cx| {
        cx.new(|cx| VaultView::new(window, cx))
    })
    .with_icon(nocterm_ui::IconName::Lock)
}

#[cfg(test)]
mod tests;
