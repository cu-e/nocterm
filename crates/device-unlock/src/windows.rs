//! Windows Hello's private RSA key is required to unwrap the vault KEK.
use crate::platform;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use nocterm_vault::{
    DeviceAvailability, DeviceCancellation, DeviceCapability, DeviceUnlockError as E,
    DeviceUnlockProvider, VaultBinding, VaultKey,
};
use serde::{Deserialize, Serialize};
use windows::{
    Security::{
        Credentials::{
            KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
        },
        Cryptography::CryptographicBuffer,
    },
    core::HSTRING,
};
use windows_future::{AsyncStatus, IAsyncOperation};
use zeroize::{Zeroize as _, Zeroizing};
fn operation<T: windows::core::RuntimeType>(
    operation: IAsyncOperation<T>,
    cancel: &DeviceCancellation,
) -> Result<T, E> {
    let started = std::time::Instant::now();
    while operation.Status().map_err(platform)? == AsyncStatus::Started {
        if cancel.is_cancelled() || started.elapsed() > std::time::Duration::from_secs(30) {
            let _ = operation.Cancel();
            return Err(E::Cancelled);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    cancel.check()?;
    operation.GetResults().map_err(platform)
}
pub(super) struct Windows;
fn delete_credential(name: &HSTRING) -> Result<(), E> {
    let operation = KeyCredentialManager::DeleteAsync(name).map_err(platform)?;
    let started = std::time::Instant::now();
    while operation.Status().map_err(platform)? == AsyncStatus::Started {
        if started.elapsed() > std::time::Duration::from_secs(5) {
            let _ = operation.Cancel();
            return Err(E::Platform("Windows Hello cleanup timed out".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    operation.GetResults().map_err(platform)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    version: u8,
    nonce: [u8; 32],
    wrap_nonce: [u8; 24],
    wrapped: Vec<u8>,
}
fn name(binding: VaultBinding, nonce: &[u8; 32]) -> HSTRING {
    let suffix = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "dev.nocterm.Nocterm.device-unlock.v1.{}.{}",
        binding.identifier(),
        suffix
    )
    .into()
}
fn token(bytes: &[u8]) -> Result<Token, E> {
    if bytes.len() > 4096 {
        return Err(E::Invalidated);
    }
    let token: Token = serde_json::from_slice(bytes).map_err(|_| E::Invalidated)?;
    if token.version != 1 || token.wrapped.len() != 48 {
        return Err(E::Invalidated);
    }
    Ok(token)
}
fn challenge(binding: VaultBinding, nonce: &[u8; 32]) -> Vec<u8> {
    let mut bytes = b"dev.nocterm.Nocterm/hello-key-wrap/v1\0".to_vec();
    bytes.extend_from_slice(&binding.vault_id);
    bytes.extend_from_slice(&binding.salt);
    bytes.extend_from_slice(nonce);
    bytes
}
fn signed_key(
    credential: &KeyCredential,
    binding: VaultBinding,
    nonce: &[u8; 32],
    cancel: &DeviceCancellation,
) -> Result<VaultKey, E> {
    cancel.check()?;
    let input =
        CryptographicBuffer::CreateFromByteArray(&challenge(binding, nonce)).map_err(platform)?;
    let result = operation(
        credential.RequestSignAsync(&input).map_err(platform)?,
        cancel,
    )?;
    match result.Status().map_err(platform)? {
        KeyCredentialStatus::Success => {}
        KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => {
            return Err(E::Cancelled);
        }
        KeyCredentialStatus::NotFound => return Err(E::Invalidated),
        _ => return Err(E::Authentication),
    }
    cancel.check()?;
    let mut bytes = windows::core::Array::new();
    CryptographicBuffer::CopyToByteArray(&result.Result().map_err(platform)?, &mut bytes)
        .map_err(platform)?;
    let signature = Zeroizing::new(bytes.as_slice().to_vec());
    bytes[..].zeroize();
    if signature.len() != 256 {
        return Err(E::Invalidated);
    }
    let mut key = VaultKey::new([0; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(Some(nonce), &signature)
        .expand(b"nocterm/vault/hello-wrapping/v1", key.as_mut())
        .map_err(|_| E::Authentication)?;
    Ok(key)
}
impl DeviceUnlockProvider for Windows {
    fn probe(&self) -> Result<DeviceCapability, E> {
        let available = KeyCredentialManager::IsSupportedAsync()
            .ok()
            .and_then(|op| operation(op, &DeviceCancellation::never()).ok())
            .unwrap_or(false);
        Ok(DeviceCapability {
            availability: if available {
                DeviceAvailability::Available
            } else {
                DeviceAvailability::NotEnrolled
            },
            label: "Windows Hello".into(),
            detail: if available {
                "Windows Hello protects this device's unlock key. Windows chooses fingerprint, face or PIN; fingerprint-only authentication cannot be enforced."
            } else {
                "Set up Windows Hello in your system settings, or use the master password."
            }
            .into(),
            enabled: false,
            armed: false,
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
        let mut wrap_nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(platform)?;
        getrandom::fill(&mut wrap_nonce).map_err(platform)?;
        let key_name = name(binding, &nonce);
        let result = operation(
            KeyCredentialManager::RequestCreateAsync(
                &key_name,
                KeyCredentialCreationOption::FailIfExists,
            )
            .map_err(platform)?,
            cancel,
        )?;
        if result.Status().map_err(platform)? != KeyCredentialStatus::Success {
            return Err(E::Authentication);
        }
        let enrollment = (|| {
            let credential = result.Credential().map_err(platform)?;
            let wrapping = signed_key(&credential, binding, &nonce, cancel)?;
            let cipher = XChaCha20Poly1305::new_from_slice(wrapping.as_slice())
                .map_err(|_| E::Authentication)?;
            let aad = challenge(binding, &nonce);
            let wrapped = cipher
                .encrypt(
                    &XNonce::from(wrap_nonce),
                    Payload {
                        msg: key.as_slice(),
                        aad: &aad,
                    },
                )
                .map_err(|_| E::Authentication)?;
            cancel.check()?;
            serde_json::to_vec(&Token {
                version: 1,
                nonce,
                wrap_nonce,
                wrapped,
            })
            .map_err(platform)
        })();
        if enrollment.is_err() {
            let _ = delete_credential(&key_name);
        }
        enrollment
    }
    fn release(
        &self,
        binding: VaultBinding,
        bytes: &[u8],
        cancel: &DeviceCancellation,
    ) -> Result<VaultKey, E> {
        cancel.check()?;
        let token = token(bytes)?;
        let result = operation(
            KeyCredentialManager::OpenAsync(&name(binding, &token.nonce)).map_err(platform)?,
            cancel,
        )?;
        if result.Status().map_err(platform)? != KeyCredentialStatus::Success {
            return Err(E::Invalidated);
        }
        let wrapping = signed_key(
            &result.Credential().map_err(platform)?,
            binding,
            &token.nonce,
            cancel,
        )?;
        let cipher = XChaCha20Poly1305::new_from_slice(wrapping.as_slice())
            .map_err(|_| E::Authentication)?;
        let aad = challenge(binding, &token.nonce);
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    &XNonce::from(token.wrap_nonce),
                    Payload {
                        msg: &token.wrapped,
                        aad: &aad,
                    },
                )
                .map_err(|_| E::Authentication)?,
        );
        cancel.check()?;
        if bytes.len() != 32 {
            return Err(E::Invalidated);
        }
        let mut key = VaultKey::new([0; 32]);
        key.copy_from_slice(&bytes);
        Ok(key)
    }
    fn remove(&self, binding: VaultBinding, bytes: &[u8]) -> Result<(), E> {
        let token = token(bytes)?;
        delete_credential(&name(binding, &token.nonce))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hello_challenge_binds_vault_rotation_and_registration() {
        let binding = VaultBinding {
            vault_id: [1; 16],
            salt: [2; 16],
        };
        let challenge_a = challenge(binding, &[3; 32]);
        assert_ne!(
            challenge_a,
            challenge(
                VaultBinding {
                    salt: [4; 16],
                    ..binding
                },
                &[3; 32]
            )
        );
        assert_ne!(
            challenge_a,
            challenge(
                VaultBinding {
                    vault_id: [4; 16],
                    ..binding
                },
                &[3; 32]
            )
        );
        assert_ne!(challenge_a, challenge(binding, &[4; 32]));
    }
    #[test]
    fn hello_public_metadata_rejects_unknown_versions_and_lengths() {
        let mut metadata = Token {
            version: 1,
            nonce: [1; 32],
            wrap_nonce: [2; 24],
            wrapped: vec![0; 48],
        };
        assert!(token(&serde_json::to_vec(&metadata).unwrap()).is_ok());
        metadata.version = 2;
        assert!(token(&serde_json::to_vec(&metadata).unwrap()).is_err());
        metadata.version = 1;
        metadata.wrapped.pop();
        assert!(token(&serde_json::to_vec(&metadata).unwrap()).is_err());
        assert!(token(&vec![0; 4097]).is_err());
    }
}
