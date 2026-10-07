//! Device-specific key release never changes the portable password envelope.
use crate::{Header, Vault, VaultError, read_file};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use zeroize::Zeroizing;

/// Key material is zeroized on drop and never exposed through formatting.
#[derive(Clone)]
pub struct VaultKey(Zeroizing<[u8; 32]>);
impl VaultKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}
impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey([REDACTED])")
    }
}
impl std::ops::Deref for VaultKey {
    type Target = [u8; 32];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for VaultKey {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultBinding {
    pub vault_id: [u8; 16],
    pub salt: [u8; 16],
}
impl VaultBinding {
    /// Public, namespaced identity; never contains key material.
    pub fn identifier(&self) -> String {
        self.vault_id
            .iter()
            .chain(self.salt.iter())
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceAvailability {
    Available,
    NoHardware,
    NotEnrolled,
    LockedOut,
    BrokerMissing,
    SigningRequired,
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct DeviceCapability {
    pub availability: DeviceAvailability,
    pub label: String,
    pub detail: String,
    /// The user turned device unlock on for this vault.
    pub enabled: bool,
    /// The device holds the key now. A session-only provider loses it when the
    /// computer restarts, until the next master password unlock.
    pub armed: bool,
    pub session_only: bool,
}
#[derive(Debug, thiserror::Error)]
pub enum DeviceUnlockError {
    #[error("Device unlock is unavailable. Use the master password.")]
    Unavailable,
    #[error("Device authentication was cancelled.")]
    Cancelled,
    #[error(
        "The device unlock registration is no longer valid. Unlock with the master password and enable it again."
    )]
    Invalidated,
    #[error(
        "Fingerprint unlock is locked after 3 failed attempts. Unlock with the master password."
    )]
    Locked,
    #[error("Device authentication failed. Use the master password or try again.")]
    Authentication,
    #[error("Device unlock failed: {0}")]
    Platform(String),
}
/// Every native operation checks the vault epoch before releasing or retaining secrets.
#[derive(Clone)]
pub struct DeviceCancellation {
    epoch: Option<(Arc<AtomicU64>, u64)>,
    observer: Option<Arc<dyn Fn(u8) + Send + Sync>>,
}
impl DeviceCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.epoch
            .as_ref()
            .is_some_and(|(epoch, expected)| epoch.load(Ordering::SeqCst) != *expected)
    }
    pub fn check(&self) -> Result<(), DeviceUnlockError> {
        if self.is_cancelled() {
            Err(DeviceUnlockError::Cancelled)
        } else {
            Ok(())
        }
    }
    /// Reports completed scan progress only while this authentication is current.
    pub fn report_attempts_remaining(&self, remaining: u8) {
        if !self.is_cancelled()
            && let Some(observer) = &self.observer
        {
            observer(remaining);
        }
    }
    pub fn never() -> Self {
        Self {
            epoch: None,
            observer: None,
        }
    }
}
/// Distinguishes a locked registration from one lost after a broker restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceRegistrationState {
    Missing,
    Ready,
    Locked,
}
/// Trusted platform adapter. Tokens contain public metadata/ciphertext only.
/// The OS or privileged broker must enforce authentication for key release.
pub trait DeviceUnlockProvider: Send + Sync {
    fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError>;
    fn enroll(
        &self,
        binding: VaultBinding,
        key: VaultKey,
        cancel: &DeviceCancellation,
    ) -> Result<Vec<u8>, DeviceUnlockError>;
    fn release(
        &self,
        binding: VaultBinding,
        token: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, DeviceUnlockError>;
    fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), DeviceUnlockError>;
    fn registration_state(
        &self,
        binding: VaultBinding,
        token: &[u8],
    ) -> Result<DeviceRegistrationState, DeviceUnlockError> {
        Ok(if self.registered(binding, token) {
            DeviceRegistrationState::Ready
        } else {
            DeviceRegistrationState::Missing
        })
    }
    /// Checks session-local registration without showing an authentication prompt.
    fn registered(&self, _binding: VaultBinding, _token: &[u8]) -> bool {
        true
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    version: u8,
    binding: VaultBinding,
    token: Vec<u8>,
}
const MAX_TOKEN: usize = 4096;
const MAX_REGISTRATION: u64 = 32768;
impl Vault {
    fn lock_device_mutation(&self) -> Result<File, VaultError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(self.path.with_extension("lock"))?;
        lock.try_lock_exclusive().map_err(|_| VaultError::Busy)?;
        Ok(lock)
    }
    fn cancellation(&self) -> DeviceCancellation {
        DeviceCancellation {
            epoch: self.guard.clone(),
            observer: None,
        }
    }
    fn device_path(&self) -> std::path::PathBuf {
        self.path.with_extension("device-unlock")
    }
    fn binding(&self) -> Result<VaultBinding, VaultError> {
        let bytes = read_file(&self.path)?;
        let header = Header::parse(&bytes)?;
        Ok(VaultBinding {
            vault_id: header.vault_id,
            salt: header.salt,
        })
    }
    fn read_registration(&self) -> Result<Registration, VaultError> {
        let file = File::open(self.device_path())?;
        let mut bytes = Vec::new();
        file.take(MAX_REGISTRATION + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_REGISTRATION {
            return Err(VaultError::Limit);
        }
        let registration: Registration =
            serde_json::from_slice(&bytes).map_err(|_| VaultError::Format)?;
        if registration.version != 1 || registration.token.len() > MAX_TOKEN {
            return Err(VaultError::Format);
        }
        Ok(registration)
    }
    pub(crate) fn probe_device_unlock(&self) -> Result<DeviceCapability, VaultError> {
        let Some(provider) = self.device.as_ref() else {
            return Ok(DeviceCapability {
                availability: DeviceAvailability::Unavailable,
                label: "Device unlock".into(),
                detail: "Device unlock is not configured. Use the master password.".into(),
                enabled: false,
                armed: false,
                session_only: false,
            });
        };
        let mut capability = provider.probe()?;
        let registration = self
            .read_registration()
            .ok()
            .filter(|r| self.binding().ok() == Some(r.binding));
        capability.enabled = registration.is_some();
        let state = if capability.availability == DeviceAvailability::Available {
            registration
                .map(|r| provider.registration_state(r.binding, &r.token))
                .transpose()?
                .unwrap_or(DeviceRegistrationState::Missing)
        } else {
            DeviceRegistrationState::Missing
        };
        capability.armed = state != DeviceRegistrationState::Missing;
        if state == DeviceRegistrationState::Locked {
            capability.availability = DeviceAvailability::LockedOut;
            capability.detail = DeviceUnlockError::Locked.to_string();
        }
        if capability.enabled
            && !capability.armed
            && capability.session_only
            && capability.availability == DeviceAvailability::Available
        {
            capability.detail = format!(
                "On. Unlock once with the master password after the computer restarts; {} unlock then works again.",
                capability.label.to_lowercase()
            );
        }
        Ok(capability)
    }
    pub(crate) fn enable_device_unlock(&mut self) -> Result<(), VaultError> {
        let provider = self.device.clone().ok_or(DeviceUnlockError::Unavailable)?;
        if provider.probe()?.availability != DeviceAvailability::Available {
            return Err(DeviceUnlockError::Unavailable.into());
        }
        let state = self.state()?;
        let binding = VaultBinding {
            vault_id: state.vault_id,
            salt: state.salt,
        };
        let key = state.kek.clone();
        let _lock = self.lock_device_mutation()?;
        if Some(read_file(&self.path)?) != self.revision {
            return Err(VaultError::Conflict);
        }
        let cancel = self.cancellation();
        cancel.check()?;
        let old = self.read_registration().ok();
        let token = provider.enroll(binding, key, &cancel)?;
        let result = (|| {
            if token.len() > MAX_TOKEN {
                return Err(VaultError::Limit);
            }
            cancel.check()?;
            let bytes = serde_json::to_vec(&Registration {
                version: 1,
                binding,
                token: token.clone(),
            })
            .map_err(|_| VaultError::Format)?;
            let directory = self.path.parent().ok_or(VaultError::Format)?;
            let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                temporary
                    .as_file()
                    .set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            temporary.write_all(&bytes)?;
            temporary.as_file().sync_all()?;
            cancel.check()?;
            if Some(read_file(&self.path)?) != self.revision {
                return Err(VaultError::Conflict);
            }
            temporary
                .persist(self.device_path())
                .map_err(|e| VaultError::Io(e.error))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = provider.remove(binding, &token);
        } else if let Some(old) = old.filter(|old| old.binding.vault_id == binding.vault_id) {
            let _ = provider.remove(old.binding, &old.token);
        }
        result
    }
    /// Session-only providers (the Linux broker) forget keys when the broker or the
    /// computer restarts. A password unlock also rearms a locked registration.
    pub(crate) fn rearm_device_unlock(&mut self) {
        let (Some(provider), Ok(registration)) = (self.device.clone(), self.read_registration())
        else {
            return;
        };
        if self.binding().ok() != Some(registration.binding)
            || matches!(
                provider.registration_state(registration.binding, &registration.token),
                Ok(DeviceRegistrationState::Ready) | Err(_)
            )
        {
            return;
        }
        if provider.probe().is_ok_and(|capability| {
            capability.session_only && capability.availability == DeviceAvailability::Available
        }) {
            let _ = self.enable_device_unlock();
        }
    }
    pub(crate) fn unlock_with_device(&mut self) -> Result<(), VaultError> {
        self.unlock_with_device_observer(None)
    }
    pub(crate) fn unlock_with_device_observer(
        &mut self,
        observer: Option<Arc<dyn Fn(u8) + Send + Sync>>,
    ) -> Result<(), VaultError> {
        self.lock();
        let provider = self.device.clone().ok_or(DeviceUnlockError::Unavailable)?;
        let registration = self.read_registration()?;
        let encrypted = read_file(&self.path)?;
        let header = Header::parse(&encrypted)?;
        let binding = VaultBinding {
            vault_id: header.vault_id,
            salt: header.salt,
        };
        if registration.binding != binding {
            return Err(DeviceUnlockError::Invalidated.into());
        }
        let cancel = self.cancellation();
        cancel.check()?;
        let cancel = DeviceCancellation { observer, ..cancel };
        let key = provider.release(binding, &registration.token, &cancel)?;
        cancel.check()?;
        // A replaced file during a native prompt cannot leave stale credentials usable.
        if read_file(&self.path)? != encrypted {
            return Err(VaultError::Conflict);
        }
        self.unlock_encrypted(encrypted, header, key)?;
        if let Err(error) = cancel.check() {
            self.lock();
            return Err(error.into());
        }
        Ok(())
    }
    pub(crate) fn disable_device_unlock(&mut self) -> Result<(), VaultError> {
        let _lock = self.lock_device_mutation()?;
        self.cancellation().check()?;
        match self.read_registration() {
            Ok(r) => {
                if r.binding.vault_id != self.binding()?.vault_id {
                    return Err(VaultError::Format);
                }
                if let Some(provider) = self.device.as_ref() {
                    provider.remove(r.binding, &r.token)?;
                }
            }
            Err(VaultError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        fs::remove_file(self.device_path())?;
        Ok(())
    }
    pub(super) fn revoke_device_after_rotation(&mut self) {
        // The new salt has already revoked every older KEK cryptographically.
        // Cleanup failure must not misreport a committed password change as failed.
        let _ = self.disable_device_unlock();
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "device_unlock/probe_tests.rs"]
mod probe_tests;
