use crate::backend::Vault;
use nocterm_vault::*;
use std::{fs, sync::Arc};
const MAX_TOKEN: usize = 4096;
impl Vault {
    fn lock_device_mutation(&self) -> Result<std::fs::File, VaultError> {
        self.store.lock()
    }
    fn cancellation(&self) -> DeviceCancellation {
        self.cancel.clone()
    }
    fn device_path(&self) -> std::path::PathBuf {
        self.store.registration_path()
    }
    fn binding(&self) -> Result<VaultBinding, VaultError> {
        Ok(EnvelopeInfo::parse(&self.store.read_envelope()?)?.binding())
    }
    fn read_registration(&self) -> Result<DeviceRegistration, VaultError> {
        self.store.registration()
    }
    pub fn probe_device_unlock(&self) -> Result<DeviceCapability, VaultError> {
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
            .filter(|r| self.binding().ok() == Some(r.binding()));
        capability.enabled = registration.is_some();
        let state = if capability.availability == DeviceAvailability::Available {
            registration
                .map(|r| provider.registration_state(r.binding(), r.token()))
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
    pub fn enable_device_unlock(&mut self) -> Result<(), VaultError> {
        let provider = self.device.clone().ok_or(DeviceUnlockError::Unavailable)?;
        if provider.probe()?.availability != DeviceAvailability::Available {
            return Err(DeviceUnlockError::Unavailable.into());
        }
        let enrollment = self.state.device_enrollment()?;
        let binding = enrollment.binding();
        let key = enrollment.take_key();
        let _lock = self.lock_device_mutation()?;
        if Some(self.store.read_envelope()?).as_deref() != self.state.revision() {
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
            let bytes = DeviceRegistration::new(binding, token.clone())?.encode()?;
            let publication =
                nocterm_core::fs::AtomicPublication::stage_private(&self.device_path(), &bytes)
                    .map_err(crate::storage::file_error)?;
            cancel.check()?;
            if Some(self.store.read_envelope()?).as_deref() != self.state.revision() {
                return Err(VaultError::Conflict);
            }
            publication.publish().map_err(crate::storage::file_error)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = provider.remove(binding, &token);
        } else if let Some(old) = old.filter(|old| old.binding().vault_id == binding.vault_id) {
            let _ = provider.remove(old.binding(), old.token());
        }
        result
    }
    /// Session-only providers (the Linux broker) forget keys when the broker or the
    /// computer restarts. A password unlock also rearms a locked registration.
    pub fn rearm_device_unlock(&mut self) {
        let (Some(provider), Ok(registration)) = (self.device.clone(), self.read_registration())
        else {
            return;
        };
        if self.binding().ok() != Some(registration.binding())
            || matches!(
                provider.registration_state(registration.binding(), registration.token()),
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
    pub fn unlock_with_device(&mut self) -> Result<(), VaultError> {
        self.unlock_with_device_observer(None)
    }
    pub fn unlock_with_device_observer(
        &mut self,
        observer: Option<Arc<dyn Fn(u8) + Send + Sync>>,
    ) -> Result<(), VaultError> {
        self.lock();
        let provider = self.device.clone().ok_or(DeviceUnlockError::Unavailable)?;
        let registration = self.read_registration()?;
        let encrypted = self.store.read_envelope()?;
        let header = EnvelopeInfo::parse(&encrypted)?;
        let binding = header.binding();
        if registration.binding() != binding {
            return Err(DeviceUnlockError::Invalidated.into());
        }
        let cancel = self.cancellation();
        cancel.check()?;
        let cancel = if let Some(observer) = observer {
            cancel.with_progress(observer)
        } else {
            cancel
        };
        let key = provider.release(binding, registration.token(), &cancel)?;
        cancel.check()?;
        // A replaced file during a native prompt cannot leave stale credentials usable.
        if self.store.read_envelope()? != encrypted {
            return Err(VaultError::Conflict);
        }
        let prepared = VaultState::prepare_device_unlock(encrypted, key)?;
        cancel.check()?;
        self.state.accept_unlock(prepared);
        if let Err(error) = cancel.check() {
            self.lock();
            return Err(error.into());
        }
        Ok(())
    }
    pub fn disable_device_unlock(&mut self) -> Result<(), VaultError> {
        let _lock = self.lock_device_mutation()?;
        self.cancellation().check()?;
        match self.read_registration() {
            Ok(r) => {
                if r.binding().vault_id != self.binding()?.vault_id {
                    return Err(VaultError::Format);
                }
                if let Some(provider) = self.device.as_ref() {
                    provider.remove(r.binding(), r.token())?;
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
