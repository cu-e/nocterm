//! The vault of the account hub: credentials kept as sealed objects on the hub.
//!
//! [`HubVault`] implements the vault port over one hub's `vault` collection. The vault's
//! password is the account's: the login password, or a passphrase of its own for an account
//! that signs in through a provider. Credentials are readable offline from the encrypted cache;
//! writes need the hub. The vault has no password of its own to create or change here, and no
//! device unlock, so those operations are [`VaultError::Unsupported`].

mod codec;
pub(crate) mod credentials;

mod idle;
mod ledger;
mod snapshot_budget;
mod unlock;
pub(crate) use idle::Idle;

use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc as progress;
use futures::future::BoxFuture;
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::{
    CredentialBinding, CredentialInfo, DeviceCapability, VaultAccess, VaultError, VaultFuture,
    VaultStatus,
};
use tokio::runtime::Handle;

pub(crate) use self::ledger::Ledger;
use self::unlock::Unlocker;
use crate::account::{Slot, operations::AccountOperations};
use crate::task::spawn;

pub struct HubVault {
    handle: Handle,
    slot: Arc<Slot>,
    operations: Arc<AccountOperations>,
    idle: Arc<Idle>,
}

impl HubVault {
    pub(crate) fn new(handle: Handle, slot: Arc<Slot>, operations: Arc<AccountOperations>) -> Self {
        Self {
            idle: Arc::new(Idle::new(handle.clone())),
            handle,
            slot,
            operations,
        }
    }

    pub(crate) fn idle(&self) -> Arc<Idle> {
        self.idle.clone()
    }

    fn run<T, F>(&self, work: impl FnOnce(Ledger) -> F + Send + 'static) -> VaultFuture<T>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, VaultError>> + Send + 'static,
    {
        let opened = match self.operations.opened() {
            Ok(opened) => opened,
            Err(error) => return Box::pin(async move { Err(native_failure(error)) }),
        };
        let source = opened.source().clone();
        let returned = source.clone();
        let idle = self.idle.clone();
        let task = spawn(&self.handle, async move {
            let result = source
                .during(async {
                    let ledger = opened.ledger().map_err(native_failure)?;
                    work(ledger).await
                })
                .await
                .map_err(native_failure)
                .and_then(|result| result);
            idle.touch(source.native());
            drop(opened);
            result
        });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(VaultError::Cancelled));
            returned.check().map_err(native_failure)?;
            result
        })
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
        let member = self.operations.current_membership();
        let slot = Arc::clone(&self.slot);
        let mut changes = slot.notice().subscribe();
        Box::pin(async move {
            let Ok(member) = member else {
                return;
            };
            loop {
                match member.with_current(|| Ok(slot.status() == VaultStatus::Unlocked)) {
                    Ok(false) => {}
                    Ok(true) | Err(_) => return,
                }
                if changes.changed().await.is_err() {
                    return;
                }
            }
        })
    }

    fn set_auto_lock(&self, duration: Duration) {
        self.idle
            .set(duration, self.operations.current_opened_source().ok());
    }

    fn lock(&self) {
        if let Ok(member) = self.operations.current_membership() {
            let _ = member.with_current(|| {
                self.idle.stop();
                self.slot.lock();
                Ok(())
            });
        }
    }

    fn create(&self, _password: Secret) -> VaultFuture<()> {
        Self::unsupported()
    }

    fn unlock(&self, password: Secret) -> VaultFuture<()> {
        let work = match self.operations.membership() {
            Ok(work) => work,
            Err(error) => return Box::pin(async move { Err(native_failure(error)) }),
        };
        let member = work.membership().clone();
        let returned = member.clone();
        let slot = match work.slot() {
            Ok(slot) => slot,
            Err(error) => return Box::pin(async move { Err(native_failure(error)) }),
        };
        let final_slot = slot.clone();
        let idle = self.idle.clone();
        let task = spawn(&self.handle, async move {
            let unlocker = Unlocker::new(work);
            let receipt = member
                .during(unlocker.unlock(password))
                .await
                .map_err(native_failure)??;
            member
                .with_current(|| slot.check_receipt(&member.subject()?, &receipt))
                .map_err(native_failure)?;
            if let Ok(native) = member.opened() {
                idle.touch(&native);
            }
            Ok(receipt)
        });
        Box::pin(async move {
            let receipt = task.await.unwrap_or(Err(VaultError::Cancelled))?;
            returned
                .with_current(|| final_slot.check_receipt(&returned.subject()?, &receipt))
                .map_err(native_failure)
        })
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
        self.run(|ledger| async move { ledger.list().await })
    }

    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        self.run(move |ledger| async move { ledger.get(id, binding).await })
    }

    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        self.run(move |ledger| async move { ledger.put(id, label, binding, secret).await })
    }

    fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        self.run(move |ledger| async move { ledger.delete(id).await })
    }
}

fn native_failure(error: nocterm_hubs_model::HubError) -> VaultError {
    match error {
        nocterm_hubs_model::HubError::KeysLocked => VaultError::Locked,
        nocterm_hubs_model::HubError::NotFound => VaultError::NotFound,
        other => ledger::vault_error(crate::store::OpError::Hub(other)),
    }
}

#[cfg(test)]
mod tests;
