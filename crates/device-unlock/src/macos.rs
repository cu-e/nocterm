//! The keychain ACL, not a process-local success flag, controls key release.
use crate::platform;
use nocterm_vault::{
    DeviceAvailability, DeviceCancellation, DeviceCapability, DeviceUnlockError as E,
    DeviceUnlockProvider, VaultBinding, VaultKey,
};
use security_framework::{
    access_control::{ProtectionMode, SecAccessControl},
    passwords::{
        AccessControlOptions, PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    },
};
const SERVICE: &str = "dev.nocterm.Nocterm.device-unlock.v1";
pub(super) struct MacOs;
fn account(binding: VaultBinding, token: &[u8]) -> Result<String, E> {
    let nonce = std::str::from_utf8(token).map_err(|_| E::Invalidated)?;
    if nonce.len() != 64
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(E::Invalidated);
    }
    Ok(format!("{}.{nonce}", binding.identifier()))
}

fn options(binding: VaultBinding, token: &[u8]) -> Result<PasswordOptions, E> {
    let mut options = PasswordOptions::new_generic_password(SERVICE, &account(binding, token)?);
    options.use_protected_keychain();
    options.set_access_synchronized(Some(false));
    Ok(options)
}
impl DeviceUnlockProvider for MacOs {
    fn probe(&self) -> Result<DeviceCapability, E> {
        let available = tid::LAContext::new()
            .can_evaluate_policy(tid::LAPolicy::DeviceOwnerAuthenticationWithBiometrics);
        Ok(DeviceCapability {
            availability: if available {
                DeviceAvailability::Available
            } else {
                DeviceAvailability::Unavailable
            },
            label: "Touch ID".into(),
            detail: if available {
                "Touch ID protects a device-local keychain entry. Adding or removing fingerprints requires enabling it again."
            } else {
                "Touch ID is unavailable or not configured. Use the master password."
            }
            .into(),
            enabled: false,
            session_only: false,
        })
    }
    fn enroll(
        &self,
        binding: VaultBinding,
        key: VaultKey,
        cancel: &DeviceCancellation,
    ) -> Result<Vec<u8>, E> {
        cancel.check()?;
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(platform)?;
        let token = nonce
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            .into_bytes();
        let mut options = options(binding, &token)?;
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
        )
        .map_err(platform)?;
        options.set_access_control(access);
        // Random per-enrollment identity avoids updating an existing weaker ACL.
        set_generic_password_options(key.as_slice(), options).map_err(platform)?;
        if let Err(error) = cancel.check() {
            let _ = self.remove(binding, &token);
            return Err(error);
        }
        Ok(token)
    }
    fn release(
        &self,
        binding: VaultBinding,
        token: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, E> {
        cancel.check()?;
        let bytes =
            zeroize::Zeroizing::new(generic_password(options(binding, token)?).map_err(|e| {
                match e.code() {
                    -128 => E::Cancelled,
                    -25300 => E::Invalidated,
                    _ => platform(e),
                }
            })?);
        cancel.check()?;
        if bytes.len() != 32 {
            return Err(E::Invalidated);
        }
        let mut key = VaultKey::new([0; 32]);
        key.copy_from_slice(&bytes);
        Ok(key)
    }
    fn remove(&self, binding: VaultBinding, token: &[u8]) -> Result<(), E> {
        match delete_generic_password_options(options(binding, token)?) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(platform(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_account_rejects_foreign_or_malformed_namespace_tokens() {
        let binding = VaultBinding {
            vault_id: [1; 16],
            salt: [2; 16],
        };
        assert!(account(binding, b"../../other-app").is_err());
        assert!(account(binding, "A".repeat(64).as_bytes()).is_err());
        let account = account(binding, "a".repeat(64).as_bytes()).unwrap();
        assert_eq!(
            account,
            format!("{}.{}", binding.identifier(), "a".repeat(64))
        );
    }
}
