use crate::Vault;
use nocterm_session::Secret;
use nocterm_vault::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU8},
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
struct Fake {
    key: Mutex<Option<VaultKey>>,
    cancelled: AtomicBool,
    corrupt: AtomicBool,
    removed: AtomicBool,
    late_cancel: AtomicBool,
    epoch: Arc<AtomicU64>,
    session_only: AtomicBool,
    token: AtomicU8,
    locked: AtomicBool,
}
impl Fake {
    fn new() -> Self {
        Self {
            key: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            corrupt: AtomicBool::new(false),
            removed: AtomicBool::new(false),
            late_cancel: AtomicBool::new(false),
            epoch: Arc::new(AtomicU64::new(0)),
            session_only: AtomicBool::new(false),
            token: AtomicU8::new(0),
            locked: AtomicBool::new(false),
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
        self.locked.store(false, Ordering::SeqCst);
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
        if self.late_cancel.load(Ordering::SeqCst) {
            self.epoch.fetch_add(1, Ordering::SeqCst);
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
    fn registration_state(
        &self,
        binding: VaultBinding,
        token: &[u8],
    ) -> Result<DeviceRegistrationState, DeviceUnlockError> {
        Ok(if self.locked.load(Ordering::SeqCst) {
            DeviceRegistrationState::Locked
        } else if self.registered(binding, token) {
            DeviceRegistrationState::Ready
        } else {
            DeviceRegistrationState::Missing
        })
    }
    fn registered(&self, _binding: VaultBinding, _token: &[u8]) -> bool {
        self.key.lock().unwrap().is_some()
    }
}
fn fixture() -> (tempfile::TempDir, Vault, Arc<Fake>) {
    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(Fake::new());
    let mut vault = Vault::with_device(temp.path().join("vault.bin"), Some(provider.clone()));
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
    vault = vault.with_cancellation(DeviceCancellation::for_epoch(provider.epoch.clone(), 0));
    assert!(vault.unlock_with_device().is_err());
    assert!(!vault.is_unlocked());
}
#[test]
fn device_roundtrip_preserves_portable_envelope_and_password_unlock() {
    let (temp, mut vault, _provider) = fixture();
    let original = fs::read(temp.path().join("vault.bin")).unwrap();
    vault.enable_device_unlock().unwrap();
    assert_eq!(original, fs::read(temp.path().join("vault.bin")).unwrap());
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
    let (temp, mut vault, provider) = fixture();
    vault.enable_device_unlock().unwrap();
    let old_key = provider.key.lock().unwrap().as_ref().unwrap().clone();
    let old_binding = EnvelopeInfo::parse(&fs::read(temp.path().join("vault.bin")).unwrap())
        .unwrap()
        .binding();
    vault
        .change_password(Secret::new("different master password"))
        .unwrap();
    assert!(provider.removed.load(Ordering::SeqCst));
    assert!(!temp.path().join("vault.device-unlock").exists());
    let encrypted = fs::read(temp.path().join("vault.bin")).unwrap();
    let header = EnvelopeInfo::parse(&encrypted).unwrap();
    assert_ne!(old_binding.salt, header.binding().salt);
    vault.lock();
    assert!(VaultState::prepare_device_unlock(encrypted, old_key).is_err());
    assert!(!vault.is_unlocked());
    vault
        .unlock(Secret::new("different master password"))
        .unwrap();
}
#[test]
fn registration_for_another_vault_cannot_delete_its_os_key() {
    let (temp, mut vault, provider) = fixture();
    vault.enable_device_unlock().unwrap();
    let registration =
        DeviceRegistration::parse(&fs::read(temp.path().join("vault.device-unlock")).unwrap())
            .unwrap();
    let mut binding = registration.binding();
    binding.vault_id = [99; 16];
    let registration = DeviceRegistration::new(binding, registration.token().to_vec()).unwrap();
    fs::write(
        temp.path().join("vault.device-unlock"),
        registration.encode().unwrap(),
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
    let (temp, mut vault, provider) = fixture();
    vault.enable_device_unlock().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(temp.path().join("vault.device-unlock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::write(temp.path().join("vault.device-unlock"), vec![0; 32769]).unwrap();
    assert!(matches!(
        DeviceRegistration::parse(&fs::read(temp.path().join("vault.device-unlock")).unwrap()),
        Err(VaultError::Limit)
    ));
    fs::remove_file(temp.path().join("vault.device-unlock")).unwrap();
    fs::create_dir(temp.path().join("vault.device-unlock")).unwrap();
    assert!(vault.enable_device_unlock().is_err());
    assert!(provider.removed.load(Ordering::SeqCst));
    assert!(vault.is_unlocked());
}
#[test]
fn cancelled_epoch_refuses_registration_before_provider_enrollment() {
    let (temp, mut vault, provider) = fixture();
    vault = vault.with_cancellation(DeviceCancellation::for_epoch(
        Arc::new(AtomicU64::new(1)),
        0,
    ));
    assert!(matches!(
        vault.enable_device_unlock(),
        Err(VaultError::Device(DeviceUnlockError::Cancelled))
    ));
    assert!(provider.key.lock().unwrap().is_none());
    assert!(!temp.path().join("vault.device-unlock").exists());
}
#[test]
fn external_revision_change_refuses_enrollment_of_stale_key() {
    let (temp, mut vault, provider) = fixture();
    let mut other = Vault::new(temp.path().join("vault.bin"));
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
    use nocterm_core::fs::{AtomicPublication, write_private_atomic};
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
    let service =
        crate::VaultService::new_with_device_unlock(&path, Duration::from_secs(60), Some(provider))
            .unwrap();
    block_on(service.create(Secret::new("long master password"))).unwrap();
    block_on(service.enable_device_unlock()).unwrap();
    // Complete password derivation and staging before the native prompt's watchdog starts.
    let replacement_path = temp.path().join("replacement.bin");
    write_private_atomic(&replacement_path, &fs::read(&path).unwrap()).unwrap();
    let mut other = Vault::new(&replacement_path);
    other.unlock(Secret::new("long master password")).unwrap();
    other
        .change_password(Secret::new("replacement master password"))
        .unwrap();
    let publication =
        AtomicPublication::stage_private(&path, &fs::read(&replacement_path).unwrap()).unwrap();
    service.lock();
    let unlock = service.unlock_with_device();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    publication.publish().unwrap();
    resume.send(()).unwrap();
    assert!(matches!(block_on(unlock), Err(VaultError::Conflict)));
    assert!(!service.is_unlocked());
    block_on(service.unlock(Secret::new("replacement master password"))).unwrap();
    assert!(service.is_unlocked());
}

#[test]
fn password_unlock_rearms_locked_registration_and_failed_password_does_not() {
    use futures::executor::block_on;
    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(Fake::new());
    provider.session_only.store(true, Ordering::SeqCst);
    let service = crate::VaultService::new_with_device_unlock(
        temp.path().join("vault.bin"),
        std::time::Duration::from_secs(60),
        Some(provider.clone()),
    )
    .unwrap();
    block_on(service.create(Secret::new("long master password"))).unwrap();
    block_on(service.enable_device_unlock()).unwrap();
    provider.locked.store(true, Ordering::SeqCst);
    service.lock();
    let capability = block_on(service.probe_device_unlock()).unwrap();
    assert!(capability.enabled && capability.armed);
    assert_eq!(capability.availability, DeviceAvailability::LockedOut);
    assert!(block_on(service.unlock(Secret::new("wrong password"))).is_err());
    block_on(service.probe_device_unlock()).unwrap();
    assert!(provider.locked.load(Ordering::SeqCst));
    block_on(service.unlock(Secret::new("long master password"))).unwrap();
    let capability = block_on(service.probe_device_unlock()).unwrap();
    assert_eq!(capability.availability, DeviceAvailability::Available);
    assert_eq!(provider.token.load(Ordering::SeqCst), 2);
}

#[test]
fn progress_observer_ignores_cancelled_epochs() {
    let epoch = Arc::new(AtomicU64::new(0));
    let (sender, receiver) = std::sync::mpsc::channel();
    let cancel =
        DeviceCancellation::for_epoch(epoch.clone(), 0).with_progress(Arc::new(move |remaining| {
            sender.send(remaining).unwrap();
        }));
    cancel.report_attempts_remaining(2);
    assert_eq!(receiver.try_recv().unwrap(), 2);
    epoch.fetch_add(1, Ordering::SeqCst);
    cancel.report_attempts_remaining(1);
    assert!(receiver.try_recv().is_err());
}

struct MissingBroker;
impl DeviceUnlockProvider for MissingBroker {
    fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError> {
        Ok(DeviceCapability {
            availability: DeviceAvailability::BrokerMissing,
            label: "Fingerprint".into(),
            detail: "Install the optional fingerprint broker.".into(),
            enabled: false,
            armed: false,
            session_only: true,
        })
    }
    fn enroll(
        &self,
        _: VaultBinding,
        _: VaultKey,
        _: &DeviceCancellation,
    ) -> Result<Vec<u8>, DeviceUnlockError> {
        panic!("missing broker cannot enroll")
    }
    fn release(
        &self,
        _: VaultBinding,
        _: &[u8],
        _: &DeviceCancellation,
    ) -> Result<VaultKey, DeviceUnlockError> {
        panic!("missing broker cannot release keys")
    }
    fn remove(&self, _: VaultBinding, _: &[u8]) -> Result<(), DeviceUnlockError> {
        Err(DeviceUnlockError::Unavailable)
    }
    fn registration_state(
        &self,
        _: VaultBinding,
        _: &[u8],
    ) -> Result<DeviceRegistrationState, DeviceUnlockError> {
        Err(DeviceUnlockError::Unavailable)
    }
}

#[test]
fn missing_broker_preserves_install_guidance_with_an_existing_registration() {
    let (temp, mut vault, _) = fixture();
    vault.enable_device_unlock().unwrap();
    vault.lock();
    vault = Vault::with_device(temp.path().join("vault.bin"), Some(Arc::new(MissingBroker)));
    let capability = vault.probe_device_unlock().unwrap();
    assert_eq!(capability.availability, DeviceAvailability::BrokerMissing);
    assert_eq!(
        capability.detail,
        "Install the optional fingerprint broker."
    );
    assert!(capability.enabled);
    assert!(!capability.armed);
    vault.unlock(Secret::new("long master password")).unwrap();
    assert!(vault.is_unlocked(), "password fallback remains usable");
}
