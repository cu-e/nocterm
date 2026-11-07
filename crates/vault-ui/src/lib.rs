//! Vault settings and authentication views with explicit lifecycle dependencies.
//! Unlock prompt facts and scan progress remain private to their original masked-control owner.
//! Owns: settings page, private masked form controls, unlock dialog and credential presentation, including the group a credential belongs to and its read-only state.
//! Does not own: encrypted format, persistence, workers or native authentication.
//! Ports used: supplied VaultAccess and SettingsHandle. Ports provided: CredentialStore, SettingsPageSpec.
//! Settings: vault. Globals: none. Invariants: hidden/locked pages clear secrets.
mod credential_row;
mod credentials;
mod settings;
mod unlock;
mod view;
pub use credentials::VaultCredentials;
use gpui_kit::{App, AppContext as _, Subscription, Window};
use nocterm_ui::SettingsPageSpec;
use nocterm_ui::UiReady;
use nocterm_vault::VaultAccess;
use nocterm_workspace::Workspace;
pub use settings::VaultSettings;
use std::{rc::Rc, sync::Arc, time::Duration};
pub use view::VaultView;

/// What a window does when the vault's keys are not on this device and cannot be
/// created here, such as signing in again to the server that holds them.
pub type KeysMissingRemedy = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Deps {
    service: Arc<dyn VaultAccess>,
    keys_missing: Option<KeysMissingRemedy>,
}
impl Deps {
    pub fn new(service: Arc<dyn VaultAccess>) -> Self {
        Self {
            service,
            keys_missing: None,
        }
    }

    /// A vault that cannot be created locally: while it does not exist, unlocking and the
    /// settings page run `remedy` instead of offering to create it.
    pub fn with_keys_missing(mut self, remedy: KeysMissingRemedy) -> Self {
        self.keys_missing = Some(remedy);
        self
    }
}
pub struct Installed {
    settings: nocterm_store::SettingsHandle,
    service: Arc<dyn VaultAccess>,
    keys_missing: Option<KeysMissingRemedy>,
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
/// Installs the application's vault: its settings, and `UnlockVault` and
/// `LockVault` for every window that has no vault of its own.
pub fn install(deps: Deps, ready: &UiReady, cx: &mut App) -> Installed {
    let auto_lock = vault_auto_lock(ready, cx);
    let installed = install_scoped(deps, ready, auto_lock, cx);
    unlock::bind_app(installed.service(), cx);
    installed
}

/// How long a vault stays open without use, read again whenever settings change; zero keeps
/// it open.
pub type AutoLock = std::rc::Rc<dyn Fn(&App) -> Duration>;

/// The auto-lock policy of the `vault` settings.
pub fn vault_auto_lock(ready: &UiReady, cx: &mut App) -> AutoLock {
    ready.settings().register::<VaultSettings>(cx);
    let policy = ready.settings().clone();
    std::rc::Rc::new(move |cx| {
        Duration::from_secs(u64::from(policy.setting::<VaultSettings>(cx).auto_lock_minutes) * 60)
    })
}

/// Installs a vault that belongs to some windows only, such as the vault of a
/// hub account. Nothing is bound to the application: a window opts in with
/// [`register_scoped`]. `auto_lock` is the vault's own lock policy.
pub fn install_scoped(deps: Deps, ready: &UiReady, auto_lock: AutoLock, cx: &mut App) -> Installed {
    ready.settings().register::<VaultSettings>(cx);
    let service = deps.service;
    let keys_missing = deps.keys_missing;
    let settings_service = service.clone();
    let settings = ready.settings().observe(cx, move |cx| {
        settings_service.set_auto_lock(auto_lock(cx));
    });
    Installed {
        settings: ready.settings().clone(),
        service,
        keys_missing,
        _settings: settings,
    }
}

/// Makes `UnlockVault` and `LockVault` in this workspace's window act on the
/// scoped vault instead of the application's.
pub fn register_scoped(installed: &Installed, workspace: &mut Workspace) {
    unlock::bind_workspace(
        installed.service(),
        installed.keys_missing.clone(),
        workspace,
    );
}

pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let service = installed.service();
    let settings = installed.settings().clone();
    let keys_missing = installed.keys_missing.clone();
    SettingsPageSpec::new("vault", "Vault", move |window, cx| {
        cx.new(|cx| {
            VaultView::new(service.clone(), settings.clone(), window, cx)
                .with_keys_missing(keys_missing.clone())
        })
    })
    .with_icon(nocterm_ui::IconName::Lock)
}
#[cfg(test)]
mod tests;
