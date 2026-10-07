use super::*;
use nocterm_session::Secret;

struct FaultingProvider(DeviceAvailability);
impl DeviceUnlockProvider for FaultingProvider {
    fn probe(&self) -> Result<DeviceCapability, DeviceUnlockError> {
        Ok(DeviceCapability {
            availability: self.0.clone(),
            label: "Fingerprint".into(),
            detail: "Provider guidance".into(),
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
        Ok(vec![1])
    }
    fn release(
        &self,
        _: VaultBinding,
        _: &[u8],
        _: &DeviceCancellation,
    ) -> Result<VaultKey, DeviceUnlockError> {
        unreachable!("passive probe does not request authentication")
    }
    fn remove(&self, _: VaultBinding, _: &[u8]) -> Result<(), DeviceUnlockError> {
        Ok(())
    }
    fn registration_state(
        &self,
        _: VaultBinding,
        _: &[u8],
    ) -> Result<DeviceRegistrationState, DeviceUnlockError> {
        Err(DeviceUnlockError::Platform(
            "Registration access denied".into(),
        ))
    }
}
#[test]
fn available_provider_registration_errors_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let mut vault = Vault::new(temp.path().join("vault.bin"));
    vault.device = Some(Arc::new(FaultingProvider(DeviceAvailability::Available)));
    vault.create(Secret::new("long master password")).unwrap();
    vault.enable_device_unlock().unwrap();
    vault.lock();
    assert!(
        matches!(vault.probe_device_unlock(), Err(VaultError::Device(DeviceUnlockError::Platform(detail))) if detail == "Registration access denied")
    );
}

#[test]
fn unavailable_provider_guidance_survives_existing_registration() {
    let temp = tempfile::tempdir().unwrap();
    let mut vault = Vault::new(temp.path().join("vault.bin"));
    vault.device = Some(Arc::new(FaultingProvider(DeviceAvailability::Available)));
    vault.create(Secret::new("long master password")).unwrap();
    vault.enable_device_unlock().unwrap();
    vault.lock();
    for availability in [
        DeviceAvailability::NoHardware,
        DeviceAvailability::NotEnrolled,
        DeviceAvailability::Unavailable,
        DeviceAvailability::BrokerMissing,
        DeviceAvailability::SigningRequired,
        DeviceAvailability::LockedOut,
    ] {
        vault.device = Some(Arc::new(FaultingProvider(availability.clone())));
        let capability = vault.probe_device_unlock().unwrap();
        assert_eq!(capability.availability, availability);
        assert_eq!(capability.detail, "Provider guidance");
        assert!(capability.enabled && !capability.armed);
    }
}
