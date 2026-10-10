use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
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
    pub fn for_epoch(epoch: Arc<AtomicU64>, expected: u64) -> Self {
        Self {
            epoch: Some((epoch, expected)),
            observer: None,
        }
    }
    pub fn with_progress(mut self, observer: Arc<dyn Fn(u8) + Send + Sync>) -> Self {
        self.observer = Some(observer);
        self
    }
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
