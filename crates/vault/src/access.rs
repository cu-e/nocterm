use crate::{CredentialBinding, CredentialInfo, DeviceCapability, VaultError};
use futures::{channel::mpsc as progress, future::BoxFuture};
use nocterm_session::{CredentialId, Secret};
use std::time::Duration;
pub type VaultFuture<T> = BoxFuture<'static, Result<T, VaultError>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VaultStatus {
    Missing,
    Locked,
    Unlocked,
}
pub trait VaultAccess: Send + Sync + 'static {
    fn is_unlocked(&self) -> bool;
    fn exists(&self) -> bool;
    fn status(&self) -> VaultStatus;
    fn revision(&self) -> u64;
    fn changed(&self, seen: u64) -> BoxFuture<'static, u64>;
    fn wait_unlocked(&self) -> BoxFuture<'static, ()>;
    fn set_auto_lock(&self, duration: Duration);
    fn lock(&self);
    fn create(&self, password: Secret) -> VaultFuture<()>;
    fn unlock(&self, password: Secret) -> VaultFuture<()>;
    fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability>;
    fn enable_device_unlock(&self) -> VaultFuture<()>;
    fn unlock_with_device(&self) -> VaultFuture<()>;
    fn unlock_with_device_progress(&self) -> (VaultFuture<()>, progress::UnboundedReceiver<u8>);
    fn disable_device_unlock(&self) -> VaultFuture<()>;
    fn change_password(&self, password: Secret) -> VaultFuture<()>;
    fn list(&self) -> VaultFuture<Vec<CredentialInfo>>;
    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret>;
    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId>;
    fn delete(&self, id: CredentialId) -> VaultFuture<()>;
}
