//! The vault of the account hub: credentials kept as sealed objects on the hub.
//!
//! [`HubVault`] implements the vault port over one hub's `vault` collection. The vault's
//! password is the account's: the login password, or a passphrase of its own for an account
//! that signs in through a provider. Credentials are readable offline from the encrypted cache;
//! writes need the hub. The vault has no password of its own to create or change here, and no
//! device unlock, so those operations are [`VaultError::Unsupported`].

mod codec;
mod groups;
mod ledger;
mod unlock;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::channel::mpsc as progress;
use futures::future::BoxFuture;
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::{
    CredentialBinding, CredentialInfo, DeviceCapability, VaultAccess, VaultError, VaultFuture,
    VaultStatus,
};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use self::ledger::Ledger;
use self::unlock::Unlocker;
use crate::account::Slot;
use crate::store::StoreOps;
use crate::task::spawn;

/// Locks the vault after a quiet period.
struct Idle {
    slot: Arc<Slot>,
    handle: Handle,
    state: Mutex<IdleState>,
}

#[derive(Default)]
struct IdleState {
    limit: Option<Duration>,
    timer: Option<AbortHandle>,
}

impl Idle {
    fn state(&self) -> std::sync::MutexGuard<'_, IdleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, limit: Duration) {
        self.state().limit = Some(limit);
        self.touch();
    }

    /// Restarts the countdown; nothing runs while the vault is locked.
    fn touch(&self) {
        let mut state = self.state();
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
        let Some(limit) = state.limit else { return };
        if self.slot.open().is_none() {
            return;
        }
        let slot = Arc::clone(&self.slot);
        state.timer = Some(
            self.handle
                .spawn(async move {
                    tokio::time::sleep(limit).await;
                    slot.lock();
                })
                .abort_handle(),
        );
    }

    fn stop(&self) {
        if let Some(timer) = self.state().timer.take() {
            timer.abort();
        }
    }
}

pub struct HubVault {
    handle: Handle,
    slot: Arc<Slot>,
    ledger: Arc<Ledger>,
    unlocker: Arc<Unlocker>,
    idle: Arc<Idle>,
}

impl HubVault {
    pub(crate) fn new(handle: Handle, slot: Arc<Slot>, ops: Arc<dyn StoreOps>) -> Self {
        Self {
            ledger: Arc::new(Ledger::new(Arc::clone(&slot), Arc::clone(&ops))),
            unlocker: Arc::new(Unlocker::new(Arc::clone(&slot), ops)),
            idle: Arc::new(Idle {
                slot: Arc::clone(&slot),
                handle: handle.clone(),
                state: Mutex::default(),
            }),
            handle,
            slot,
        }
    }

    /// Runs `work` on the adapter's threads, then counts it as use of the vault.
    fn run<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = Result<T, VaultError>> + Send + 'static,
    ) -> VaultFuture<T> {
        let idle = Arc::clone(&self.idle);
        let task = spawn(&self.handle, async move {
            let result = work.await;
            idle.touch();
            result
        });
        Box::pin(async move { task.await.unwrap_or(Err(VaultError::Cancelled)) })
    }

    fn unsupported<T: Send + 'static>() -> VaultFuture<T> {
        Box::pin(async { Err(VaultError::Unsupported) })
    }
}

impl VaultAccess for HubVault {
    fn is_unlocked(&self) -> bool {
        self.slot.status() == VaultStatus::Unlocked
    }

    fn exists(&self) -> bool {
        self.slot.status() != VaultStatus::Missing
    }

    fn status(&self) -> VaultStatus {
        self.slot.status()
    }

    fn revision(&self) -> u64 {
        self.slot.notice().count()
    }

    fn changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let mut changes = self.slot.notice().subscribe();
        Box::pin(async move {
            loop {
                let now = *changes.borrow_and_update();
                if now != seen || changes.changed().await.is_err() {
                    return now;
                }
            }
        })
    }

    fn wait_unlocked(&self) -> BoxFuture<'static, ()> {
        let slot = Arc::clone(&self.slot);
        let mut changes = slot.notice().subscribe();
        Box::pin(async move {
            while slot.status() != VaultStatus::Unlocked {
                if changes.changed().await.is_err() {
                    return;
                }
            }
        })
    }

    fn set_auto_lock(&self, duration: Duration) {
        self.idle.set(duration);
    }

    fn lock(&self) {
        self.idle.stop();
        self.slot.lock();
    }

    fn create(&self, _password: Secret) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn unlock(&self, password: Secret) -> VaultFuture<()> {
        let unlocker = Arc::clone(&self.unlocker);
        self.run(async move { unlocker.unlock(password).await })
    }

    fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability> {
        Self::unsupported()
    }

    fn enable_device_unlock(&self) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn unlock_with_device(&self) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn unlock_with_device_progress(&self) -> (VaultFuture<()>, progress::UnboundedReceiver<u8>) {
        let (_, steps) = progress::unbounded();
        (Self::unsupported(), steps)
    }

    fn disable_device_unlock(&self) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn change_password(&self, _password: Secret) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn list(&self) -> VaultFuture<Vec<CredentialInfo>> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move { ledger.list().await })
    }

    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move { ledger.get(id, binding).await })
    }

    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move { ledger.put(id, label, binding, secret).await })
    }

    fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        let ledger = Arc::clone(&self.ledger);
        self.run(async move { ledger.delete(id).await })
    }
}

#[cfg(test)]
mod tests;
