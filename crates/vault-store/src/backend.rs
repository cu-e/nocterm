use crate::{entropy, storage::FileStore};
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::*;
use std::{path::PathBuf, sync::Arc};
pub struct Vault {
    store: FileStore,
    state: VaultState,
    device: Option<Arc<dyn DeviceUnlockProvider>>,
    cancel: DeviceCancellation,
}
impl Vault {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_device(path, None)
    }
    pub fn with_device(
        path: impl Into<PathBuf>,
        device: Option<Arc<dyn DeviceUnlockProvider>>,
    ) -> Self {
        Self {
            store: FileStore::new(path.into()),
            state: VaultState::locked(),
            device,
            cancel: DeviceCancellation::never(),
        }
    }
    pub fn with_cancellation(mut self, cancel: DeviceCancellation) -> Self {
        self.cancel = cancel;
        self
    }
    pub(crate) fn set_cancellation(&mut self, cancel: DeviceCancellation) {
        self.cancel = cancel;
    }
    pub fn exists(&self) -> bool {
        self.store.exists()
    }
    pub fn is_unlocked(&self) -> bool {
        self.state.is_unlocked()
    }
    pub fn lock(&mut self) {
        self.state.lock();
    }
    pub fn create(&mut self, password: Secret) -> Result<(), VaultError> {
        if self.exists() {
            return Err(VaultError::AlreadyExists);
        }
        let prepared = self
            .state
            .prepare_create(password, entropy::create_material()?)?;
        self.commit(prepared)
    }
    pub fn unlock(&mut self, password: Secret) -> Result<(), VaultError> {
        self.lock();
        let candidate = VaultState::prepare_unlock(self.store.read_envelope()?, password)?;
        self.cancel.check()?;
        self.state.accept_unlock(candidate);
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        self.state.list()
    }
    pub fn get(&self, id: CredentialId, binding: &CredentialBinding) -> Result<Secret, VaultError> {
        self.state.get(id, binding)
    }
    pub fn put(
        &mut self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> Result<CredentialId, VaultError> {
        let id = id.map(Ok).unwrap_or_else(entropy::credential_id)?;
        let candidate = self
            .state
            .prepare_put(id, label, binding, secret, entropy::nonces()?)?;
        self.commit(candidate)?;
        Ok(id)
    }
    pub fn delete(&mut self, id: CredentialId) -> Result<(), VaultError> {
        let candidate = self.state.prepare_delete(id, entropy::nonces()?)?;
        self.commit(candidate)
    }
    pub fn change_password(&mut self, password: Secret) -> Result<(), VaultError> {
        let candidate = self
            .state
            .prepare_password_change(password, entropy::rotation_material()?)?;
        self.commit(candidate)?;
        self.revoke_device_after_rotation();
        Ok(())
    }
    fn commit(&mut self, candidate: PreparedWrite) -> Result<(), VaultError> {
        self.store.commit(&candidate, &self.cancel)?;
        self.state.accept_write(candidate);
        Ok(())
    }
}

#[path = "device.rs"]
mod device;

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("unlocked", &self.is_unlocked())
            .finish_non_exhaustive()
    }
}
