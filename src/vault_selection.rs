//! The vault the application uses right now.
//!
//! The device's own vault file and the vault of a hub account are two vaults behind the same
//! [`VaultAccess`]. Everything that holds the vault (the credential provider, the vault page,
//! the key editor) holds a [`SelectedVault`]; choosing another account moves what it
//! delegates to without restarting anything that observes it.
use futures::{FutureExt as _, channel::mpsc as progress, future::BoxFuture};
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::{
    CredentialBinding, CredentialInfo, DeviceCapability, VaultAccess, VaultFuture, VaultStatus,
};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// A vault that delegates to the one selected last.
///
/// Its revision keeps rising across a switch, so an observer waiting on `changed` never misses
/// the move to another vault.
#[derive(Clone)]
pub(crate) struct SelectedVault {
    shared: Arc<Shared>,
}

struct Shared {
    /// The vault the application started with: the one of this device.
    home: Arc<dyn VaultAccess>,
    current: Mutex<Current>,
    switched: event_listener::Event,
}

struct Current {
    vault: Arc<dyn VaultAccess>,
    /// Added to the vault's own revision: what the vaults selected before reached, plus one
    /// for each move.
    base: u64,
    auto_lock: Option<Duration>,
}

impl SelectedVault {
    pub(crate) fn new(vault: Arc<dyn VaultAccess>) -> Self {
        Self {
            shared: Arc::new(Shared {
                home: Arc::clone(&vault),
                current: Mutex::new(Current {
                    vault,
                    base: 0,
                    auto_lock: None,
                }),
                switched: event_listener::Event::new(),
            }),
        }
    }

    /// Makes `vault` the one in use and locks the one it replaces: unlocking a vault never
    /// carries over to another. Selecting the vault already in use changes nothing.
    pub(crate) fn select(&self, vault: Arc<dyn VaultAccess>) {
        {
            let mut current = self.current();
            if Arc::ptr_eq(&current.vault, &vault) {
                return;
            }
            current.vault.lock();
            current.base += current.vault.revision() + 1;
            if let Some(duration) = current.auto_lock {
                vault.set_auto_lock(duration);
            }
            current.vault = vault;
        }
        self.shared.switched.notify(usize::MAX);
    }

    /// Goes back to the vault of this device.
    pub(crate) fn select_home(&self) {
        self.select(Arc::clone(&self.shared.home));
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Current> {
        self.shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn vault(&self) -> Arc<dyn VaultAccess> {
        Arc::clone(&self.current().vault)
    }
}

impl Shared {
    fn snapshot(&self) -> (Arc<dyn VaultAccess>, u64) {
        let current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        (Arc::clone(&current.vault), current.base)
    }
}

impl VaultAccess for SelectedVault {
    fn is_unlocked(&self) -> bool {
        self.vault().is_unlocked()
    }

    fn exists(&self) -> bool {
        self.vault().exists()
    }

    fn status(&self) -> VaultStatus {
        self.vault().status()
    }

    fn revision(&self) -> u64 {
        let current = self.current();
        current.base + current.vault.revision()
    }

    fn changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let shared = Arc::clone(&self.shared);
        async move {
            loop {
                let moved = shared.switched.listen();
                let (vault, base) = shared.snapshot();
                let now = base + vault.revision();
                if now != seen {
                    return now;
                }
                // `seen == base + revision`, so the vault's own view of it is the difference.
                let own = vault.changed(seen - base);
                // Either way the loop reads the selection again: a switch locks the vault it
                // replaces, which wakes `own` before the new base is in place.
                futures::select_biased! {
                    _ = own.fuse() => {}
                    () = moved.fuse() => {}
                }
            }
        }
        .boxed()
    }

    fn wait_unlocked(&self) -> BoxFuture<'static, ()> {
        let shared = Arc::clone(&self.shared);
        async move {
            loop {
                let moved = shared.switched.listen();
                let (vault, _) = shared.snapshot();
                if vault.is_unlocked() {
                    return;
                }
                futures::select_biased! {
                    () = vault.wait_unlocked().fuse() => return,
                    () = moved.fuse() => {}
                }
            }
        }
        .boxed()
    }

    fn set_auto_lock(&self, duration: Duration) {
        let mut current = self.current();
        current.auto_lock = Some(duration);
        current.vault.set_auto_lock(duration);
    }

    fn lock(&self) {
        self.vault().lock();
    }

    fn create(&self, password: Secret) -> VaultFuture<()> {
        self.vault().create(password)
    }

    fn unlock(&self, password: Secret) -> VaultFuture<()> {
        self.vault().unlock(password)
    }

    fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability> {
        self.vault().probe_device_unlock()
    }

    fn enable_device_unlock(&self) -> VaultFuture<()> {
        self.vault().enable_device_unlock()
    }

    fn unlock_with_device(&self) -> VaultFuture<()> {
        self.vault().unlock_with_device()
    }

    fn unlock_with_device_progress(&self) -> (VaultFuture<()>, progress::UnboundedReceiver<u8>) {
        self.vault().unlock_with_device_progress()
    }

    fn disable_device_unlock(&self) -> VaultFuture<()> {
        self.vault().disable_device_unlock()
    }

    fn change_password(&self, password: Secret) -> VaultFuture<()> {
        self.vault().change_password(password)
    }

    fn list(&self) -> VaultFuture<Vec<CredentialInfo>> {
        self.vault().list()
    }

    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        self.vault().get(id, binding)
    }

    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        self.vault().put(id, label, binding, secret)
    }

    fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        self.vault().delete(id)
    }
}

#[cfg(test)]
mod tests;
