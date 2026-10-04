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
    #[error("Device authentication failed. Use the master password or try again.")]
    Authentication,
    #[error("Device unlock failed: {0}")]
    Platform(String),
}
/// Every native operation checks the vault epoch before releasing or retaining secrets.
#[derive(Clone)]
pub struct DeviceCancellation {
    epoch: Option<(Arc<AtomicU64>, u64)>,
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
    pub fn never() -> Self {
        Self { epoch: None }
    }
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
        capability.armed = registration.is_some_and(|r| provider.registered(r.binding, &r.token));
        if capability.enabled && !capability.armed && capability.session_only {
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
    /// computer restarts. A password unlock re-registers a device the user enabled.
    pub(crate) fn rearm_device_unlock(&mut self) {
        let (Some(provider), Ok(registration)) = (self.device.clone(), self.read_registration())
        else {
            return;
        };
        if self.binding().ok() != Some(registration.binding)
            || provider.registered(registration.binding, &registration.token)
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
mod tests {
    use super::*;
    use nocterm_session::Secret;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8},
    };
    struct Fake {
        key: Mutex<Option<VaultKey>>,
        cancelled: AtomicBool,
        corrupt: AtomicBool,
        removed: AtomicBool,
        late_cancel: AtomicBool,
        session_only: AtomicBool,
        token: AtomicU8,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                key: Mutex::new(None),
                cancelled: AtomicBool::new(false),
                corrupt: AtomicBool::new(false),
                removed: AtomicBool::new(false),
                late_cancel: AtomicBool::new(false),
                session_only: AtomicBool::new(false),
                token: AtomicU8::new(0),
            }
        }
    }
    impl DeviceUnlockProvider for Fake {
        fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError> {
            Ok(DeviceCapability {
                availability: DeviceAvailability::Available,
                label: "Test device".into(),
                detail: String::new(),
                enabled: false,
                armed: false,
                session_only: self.session_only.load(Ordering::SeqCst),
            })
        }
        fn enroll(
            &self,
            _binding: VaultBinding,
            key: VaultKey,
            _cancel: &DeviceCancellation,
        ) -> Result<Vec<u8>, DeviceUnlockError> {
            *self.key.lock().unwrap() = Some(key);
            Ok(vec![self.token.fetch_add(1, Ordering::SeqCst) + 1])
        }
        fn release(
            &self,
            _binding: VaultBinding,
            _token: &[u8],
            cancel: &DeviceCancellation,
        ) -> Result<VaultKey, DeviceUnlockError> {
            cancel.check()?;
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(DeviceUnlockError::Cancelled);
            }
            let mut key = self
                .key
                .lock()
                .unwrap()
                .as_ref()
                .ok_or(DeviceUnlockError::Invalidated)?
                .clone();
            if self.corrupt.load(Ordering::SeqCst) {
                key[0] ^= 1;
            }
            if self.late_cancel.load(Ordering::SeqCst)
                && let Some((epoch, _)) = &cancel.epoch
            {
                epoch.fetch_add(1, Ordering::SeqCst);
            }
            Ok(key)
        }
        fn remove(&self, _binding: VaultBinding, token: &[u8]) -> Result<(), DeviceUnlockError> {
            self.removed.store(true, Ordering::SeqCst);
            if token == [self.token.load(Ordering::SeqCst)] {
                *self.key.lock().unwrap() = None;
            }
            Ok(())
        }
        fn registered(&self, _binding: VaultBinding, _token: &[u8]) -> bool {
            self.key.lock().unwrap().is_some()
        }
    }
    fn fixture() -> (tempfile::TempDir, Vault, Arc<Fake>) {
        let temp = tempfile::tempdir().unwrap();
        let mut vault = Vault::new(temp.path().join("vault.bin"));
        let provider = Arc::new(Fake::new());
        vault.device = Some(provider.clone());
        vault.create(Secret::new("long master password")).unwrap();
        (temp, vault, provider)
    }
    #[test]
    fn key_formatting_is_redacted() {
        assert_eq!(
            format!("{:?}", VaultKey::new([42; 32])),
            "VaultKey([REDACTED])"
        );
    }
    #[test]
    fn password_unlock_rearms_a_session_only_device() {
        let (_temp, mut vault, provider) = fixture();
        provider.session_only.store(true, Ordering::SeqCst);
        vault.enable_device_unlock().unwrap();
        // A broker restart forgets every session key.
        *provider.key.lock().unwrap() = None;
        vault.lock();
        // The user's choice survives; only the key has to be armed again.
        let capability = vault.probe_device_unlock().unwrap();
        assert!(capability.enabled && !capability.armed);
        vault.unlock(Secret::new("long master password")).unwrap();
        vault.rearm_device_unlock();
        let capability = vault.probe_device_unlock().unwrap();
        assert!(capability.enabled && capability.armed);
        vault.lock();
        vault.unlock_with_device().unwrap();
        assert!(vault.is_unlocked());
    }
    #[test]
    fn late_success_cannot_reopen_after_lock_epoch_changes() {
        let (_temp, mut vault, provider) = fixture();
        vault.enable_device_unlock().unwrap();
        provider.late_cancel.store(true, Ordering::SeqCst);
        vault.guard = Some((Arc::new(AtomicU64::new(0)), 0));
        assert!(vault.unlock_with_device().is_err());
        assert!(!vault.is_unlocked());
    }
    #[test]
    fn device_roundtrip_preserves_portable_envelope_and_password_unlock() {
        let (_temp, mut vault, _provider) = fixture();
        let original = fs::read(&vault.path).unwrap();
        vault.enable_device_unlock().unwrap();
        assert_eq!(original, fs::read(&vault.path).unwrap());
        vault.lock();
        vault.unlock_with_device().unwrap();
        assert!(vault.is_unlocked());
        vault.lock();
        vault.unlock(Secret::new("long master password")).unwrap();
        assert!(vault.is_unlocked());
    }
    #[test]
    fn wrong_or_cancelled_device_key_never_leaves_vault_unlocked() {
        let (_temp, mut vault, provider) = fixture();
        vault.enable_device_unlock().unwrap();
        provider.corrupt.store(true, Ordering::SeqCst);
        assert!(vault.unlock_with_device().is_err());
        assert!(!vault.is_unlocked());
        provider.cancelled.store(true, Ordering::SeqCst);
        assert!(matches!(
            vault.unlock_with_device(),
            Err(VaultError::Device(DeviceUnlockError::Cancelled))
        ));
        assert!(!vault.is_unlocked());
    }
    #[test]
    fn password_rotation_revokes_device_registration_and_old_key() {
        let (_temp, mut vault, provider) = fixture();
        vault.enable_device_unlock().unwrap();
        let old_key = provider.key.lock().unwrap().as_ref().unwrap().clone();
        let old_binding = vault.binding().unwrap();
        vault
            .change_password(Secret::new("different master password"))
            .unwrap();
        assert!(provider.removed.load(Ordering::SeqCst));
        assert!(!vault.device_path().exists());
        let encrypted = read_file(&vault.path).unwrap();
        let header = Header::parse(&encrypted).unwrap();
        assert_ne!(old_binding.salt, header.salt);
        vault.lock();
        assert!(vault.unlock_encrypted(encrypted, header, old_key).is_err());
        assert!(!vault.is_unlocked());
        vault
            .unlock(Secret::new("different master password"))
            .unwrap();
    }
    #[test]
    fn registration_for_another_vault_cannot_delete_its_os_key() {
        let (_temp, mut vault, provider) = fixture();
        vault.enable_device_unlock().unwrap();
        let mut registration = vault.read_registration().unwrap();
        registration.binding.vault_id = [99; 16];
        fs::write(
            vault.device_path(),
            serde_json::to_vec(&registration).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            vault.disable_device_unlock(),
            Err(VaultError::Format)
        ));
        assert!(!provider.removed.load(Ordering::SeqCst));
        assert!(vault.unlock_with_device().is_err());
    }
    #[test]
    fn sidecar_is_bounded_private_and_failed_enrollment_cleans_native_key() {
        let (_temp, mut vault, provider) = fixture();
        vault.enable_device_unlock().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(vault.device_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::write(vault.device_path(), vec![0; MAX_REGISTRATION as usize + 1]).unwrap();
        assert!(matches!(vault.read_registration(), Err(VaultError::Limit)));
        fs::remove_file(vault.device_path()).unwrap();
        fs::create_dir(vault.device_path()).unwrap();
        assert!(vault.enable_device_unlock().is_err());
        assert!(provider.removed.load(Ordering::SeqCst));
        assert!(vault.is_unlocked());
    }
    #[test]
    fn cancelled_epoch_refuses_registration_before_provider_enrollment() {
        let (_temp, mut vault, provider) = fixture();
        vault.guard = Some((Arc::new(AtomicU64::new(1)), 0));
        assert!(matches!(
            vault.enable_device_unlock(),
            Err(VaultError::Device(DeviceUnlockError::Cancelled))
        ));
        assert!(provider.key.lock().unwrap().is_none());
        assert!(!vault.device_path().exists());
    }
    #[test]
    fn external_revision_change_refuses_enrollment_of_stale_key() {
        let (_temp, mut vault, provider) = fixture();
        let mut other = Vault::new(vault.path.clone());
        other.unlock(Secret::new("long master password")).unwrap();
        other
            .change_password(Secret::new("another master password"))
            .unwrap();
        assert!(matches!(
            vault.enable_device_unlock(),
            Err(VaultError::Conflict)
        ));
        assert!(provider.key.lock().unwrap().is_none());
    }

    struct Prompt {
        fake: Fake,
        entered: std::sync::mpsc::Sender<()>,
        resume: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl DeviceUnlockProvider for Prompt {
        fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError> {
            self.fake.probe()
        }
        fn enroll(
            &self,
            binding: VaultBinding,
            key: VaultKey,
            cancel: &DeviceCancellation,
        ) -> Result<Vec<u8>, DeviceUnlockError> {
            self.fake.enroll(binding, key, cancel)
        }
        fn release(
            &self,
            binding: VaultBinding,
            token: &[u8],
            cancel: &DeviceCancellation,
        ) -> Result<VaultKey, DeviceUnlockError> {
            let key = self.fake.release(binding, token, cancel)?;
            self.entered.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            // A native provider can complete after cancellation. The vault must
            // independently reject the returned key at its own commit boundary.
            Ok(key)
        }
        fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), DeviceUnlockError> {
            self.fake.remove(binding, token)
        }
    }

    #[test]
    fn worker_lock_during_native_prompt_rejects_success_and_queued_credentials() {
        use futures::executor::block_on;
        use std::{sync::mpsc, time::Duration};
        let temp = tempfile::tempdir().unwrap();
        let (entered, ready) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let provider = Arc::new(Prompt {
            fake: Fake::new(),
            entered,
            resume: Mutex::new(wait),
        });
        let service = crate::VaultService::new_with_device_unlock(
            temp.path().join("vault.bin"),
            Duration::from_secs(60),
            Some(provider),
        )
        .unwrap();
        block_on(service.create(Secret::new("long master password"))).unwrap();
        block_on(service.enable_device_unlock()).unwrap();
        service.lock();
        let unlock = service.unlock_with_device();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let queued_credentials = service.list();
        service.lock();
        assert!(!service.is_unlocked());
        resume.send(()).unwrap();
        assert!(matches!(block_on(unlock), Err(VaultError::Cancelled)));
        assert!(matches!(
            block_on(queued_credentials),
            Err(VaultError::Cancelled)
        ));
        assert!(!service.is_unlocked());
        block_on(service.unlock(Secret::new("long master password"))).unwrap();
        assert!(service.is_unlocked(), "password fallback still works");
    }

    #[test]
    fn vault_replacement_during_native_prompt_refuses_stale_decrypted_state() {
        use futures::executor::block_on;
        use std::{sync::mpsc, time::Duration};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vault.bin");
        let (entered, ready) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let provider = Arc::new(Prompt {
            fake: Fake::new(),
            entered,
            resume: Mutex::new(wait),
        });
        let service = crate::VaultService::new_with_device_unlock(
            &path,
            Duration::from_secs(60),
            Some(provider),
        )
        .unwrap();
        block_on(service.create(Secret::new("long master password"))).unwrap();
        block_on(service.enable_device_unlock()).unwrap();
        service.lock();
        let unlock = service.unlock_with_device();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut other = Vault::new(&path);
        other.unlock(Secret::new("long master password")).unwrap();
        other
            .change_password(Secret::new("replacement master password"))
            .unwrap();
        resume.send(()).unwrap();
        assert!(matches!(block_on(unlock), Err(VaultError::Conflict)));
        assert!(!service.is_unlocked());
        block_on(service.unlock(Secret::new("replacement master password"))).unwrap();
        assert!(service.is_unlocked());
    }
}
