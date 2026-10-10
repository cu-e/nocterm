use crate::VaultService;
use futures::{channel::mpsc as progress, future::BoxFuture};
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::*;
use std::time::Duration;
impl VaultAccess for VaultService {
    fn is_unlocked(&self) -> bool {
        VaultService::is_unlocked(self)
    }
    fn exists(&self) -> bool {
        VaultService::exists(self)
    }
    fn status(&self) -> VaultStatus {
        VaultService::status(self)
    }
    fn revision(&self) -> u64 {
        VaultService::revision(self)
    }
    fn changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        VaultService::changed(self, seen)
    }
    fn wait_unlocked(&self) -> BoxFuture<'static, ()> {
        VaultService::wait_unlocked(self)
    }
    fn set_auto_lock(&self, duration: Duration) {
        VaultService::set_auto_lock(self, duration)
    }
    fn lock(&self) {
        VaultService::lock(self)
    }
    fn create(&self, password: Secret) -> VaultFuture<()> {
        VaultService::create(self, password)
    }
    fn unlock(&self, password: Secret) -> VaultFuture<()> {
        VaultService::unlock(self, password)
    }
    fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability> {
        VaultService::probe_device_unlock(self)
    }
    fn enable_device_unlock(&self) -> VaultFuture<()> {
        VaultService::enable_device_unlock(self)
    }
    fn unlock_with_device(&self) -> VaultFuture<()> {
        VaultService::unlock_with_device(self)
    }
    fn unlock_with_device_progress(&self) -> (VaultFuture<()>, progress::UnboundedReceiver<u8>) {
        VaultService::unlock_with_device_progress(self)
    }
    fn disable_device_unlock(&self) -> VaultFuture<()> {
        VaultService::disable_device_unlock(self)
    }
    fn change_password(&self, password: Secret) -> VaultFuture<()> {
        VaultService::change_password(self, password)
    }
    fn list(&self) -> VaultFuture<Vec<CredentialInfo>> {
        VaultService::list(self)
    }
    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        VaultService::get(self, id, binding)
    }
    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        VaultService::put(self, id, label, binding, secret)
    }
    fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        VaultService::delete(self, id)
    }
}
