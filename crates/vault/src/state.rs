use crate::{
    CreateMaterial, CredentialBinding, CredentialInfo, RotationMaterial, SealNonces, VaultBinding,
    VaultError, VaultKey,
};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use nocterm_session::{CredentialId, Secret};
use std::fmt;
use zeroize::{Zeroize as _, Zeroizing};
mod format;
use format::{Header, cipher, decode_records, derive, encrypt};
const MAGIC: &[u8; 8] = b"NOCVAULT";
const HEADER_SIZE: usize = 108;
const WRAPPED_SIZE: usize = 48;
const MAX_PAYLOAD: usize = 1_048_576;
const MAX_FILE: usize = HEADER_SIZE + WRAPPED_SIZE + MAX_PAYLOAD + 16;
const MEMORY_KIB: u32 = 65536;
const ITERATIONS: u32 = 3;
const LANES: u32 = 4;
const MAX_RECORDS: usize = 1024;

struct Record {
    info: CredentialInfo,
    secret: Secret,
}
impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("id", &self.info.id)
            .finish_non_exhaustive()
    }
}
impl Drop for Record {
    fn drop(&mut self) {
        self.info.label.zeroize();
    }
}

struct Unlocked {
    kek: VaultKey,
    dek: Zeroizing<[u8; 32]>,
    salt: [u8; 16],
    vault_id: [u8; 16],
    records: Vec<Record>,
}

pub struct VaultState {
    revision: Option<Vec<u8>>,
    unlocked: Option<Unlocked>,
}
/// An unpublished encrypted revision with its private replacement state.
/// Dropping it keeps accepted state intact; acceptance follows successful publication.
pub struct PreparedWrite {
    encrypted: Vec<u8>,
    replacement: Unlocked,
    expected: Option<Vec<u8>>,
}
impl PreparedWrite {
    pub fn bytes(&self) -> &[u8] {
        &self.encrypted
    }
    pub fn expected_revision(&self) -> Option<&[u8]> {
        self.expected.as_deref()
    }
}
pub struct PreparedUnlock {
    encrypted: Vec<u8>,
    replacement: Unlocked,
}
pub struct DeviceEnrollment {
    binding: VaultBinding,
    key: VaultKey,
}
impl DeviceEnrollment {
    pub fn binding(&self) -> VaultBinding {
        self.binding
    }
    pub fn take_key(self) -> VaultKey {
        self.key
    }
}
pub struct EnvelopeInfo {
    binding: VaultBinding,
}
impl EnvelopeInfo {
    pub fn parse(bytes: &[u8]) -> Result<Self, VaultError> {
        let h = Header::parse(bytes)?;
        Ok(Self {
            binding: VaultBinding {
                vault_id: h.vault_id,
                salt: h.salt,
            },
        })
    }
    pub fn binding(&self) -> VaultBinding {
        self.binding
    }
}
impl VaultState {
    pub fn locked() -> Self {
        Self {
            revision: None,
            unlocked: None,
        }
    }
    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }
    pub fn lock(&mut self) {
        self.unlocked = None;
        self.revision = None;
    }
    pub fn revision(&self) -> Option<&[u8]> {
        self.revision.as_deref()
    }
    pub fn prepare_create(
        &self,
        password: Secret,
        material: CreateMaterial,
    ) -> Result<PreparedWrite, VaultError> {
        validate_password(&password)?;
        let (salt, dek, vault_id, nonces) = material.into_parts();
        let state = Unlocked {
            kek: derive(&password, &salt)?,
            dek: Zeroizing::new(*dek),
            salt,
            vault_id,
            records: Vec::new(),
        };
        Ok(PreparedWrite {
            encrypted: encrypt(&state, nonces)?,
            replacement: state,
            expected: None,
        })
    }
    pub fn prepare_unlock(
        encrypted: Vec<u8>,
        password: Secret,
    ) -> Result<PreparedUnlock, VaultError> {
        let header = Header::parse(&encrypted)?;
        let kek = derive(&password, &header.salt)?;
        Self::decrypt(encrypted, header, kek)
    }
    pub fn prepare_device_unlock(
        encrypted: Vec<u8>,
        kek: VaultKey,
    ) -> Result<PreparedUnlock, VaultError> {
        let header = Header::parse(&encrypted)?;
        Self::decrypt(encrypted, header, kek)
    }
    fn decrypt(
        encrypted: Vec<u8>,
        header: Header,
        kek: VaultKey,
    ) -> Result<PreparedUnlock, VaultError> {
        let wrapped = &encrypted[HEADER_SIZE..HEADER_SIZE + WRAPPED_SIZE];
        let key = Zeroizing::new(
            cipher(&kek)?
                .decrypt(
                    &XNonce::from(header.wrap_nonce),
                    Payload {
                        msg: wrapped,
                        aad: &encrypted[..HEADER_SIZE],
                    },
                )
                .map_err(|_| VaultError::Authentication)?,
        );
        if key.len() != 32 {
            return Err(VaultError::Format);
        }
        let mut dek = Zeroizing::new([0; 32]);
        dek.copy_from_slice(&key);
        let plaintext = Zeroizing::new(
            cipher(&dek)?
                .decrypt(
                    &XNonce::from(header.payload_nonce),
                    Payload {
                        msg: &encrypted[HEADER_SIZE + WRAPPED_SIZE..],
                        aad: &encrypted[..HEADER_SIZE + WRAPPED_SIZE],
                    },
                )
                .map_err(|_| VaultError::Authentication)?,
        );
        let records = decode_records(&plaintext)?;
        validate_records(&records)?;
        let replacement = Unlocked {
            kek,
            dek,
            salt: header.salt,
            vault_id: header.vault_id,
            records,
        };
        Ok(PreparedUnlock {
            encrypted,
            replacement,
        })
    }

    pub fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        Ok(self
            .state()?
            .records
            .iter()
            .map(|record| record.info.clone())
            .collect())
    }
    pub fn get(&self, id: CredentialId, binding: &CredentialBinding) -> Result<Secret, VaultError> {
        let record = self
            .state()?
            .records
            .iter()
            .find(|record| record.info.id == id)
            .ok_or(VaultError::NotFound)?;
        if &record.info.binding != binding {
            return Err(VaultError::Binding);
        }
        Ok(record.secret.clone())
    }
    pub fn prepare_put(
        &self,
        id: CredentialId,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
        nonces: SealNonces,
    ) -> Result<PreparedWrite, VaultError> {
        if label.len() > 1024 || secret.expose().len() > 65536 {
            return Err(VaultError::Limit);
        }
        if let Some(record) = self
            .state()?
            .records
            .iter()
            .find(|record| record.info.id == id)
            && record.info.binding != binding
        {
            return Err(VaultError::Binding);
        }
        let mut records = clone_records(&self.state()?.records);
        let new = Record {
            info: CredentialInfo { id, label, binding },
            secret,
        };
        if let Some(index) = records.iter().position(|record| record.info.id == id) {
            records[index] = new;
        } else {
            records.push(new);
        }
        validate_records(&records)?;
        self.prepare_records(records, nonces)
    }
    pub fn prepare_delete(
        &self,
        id: CredentialId,
        nonces: SealNonces,
    ) -> Result<PreparedWrite, VaultError> {
        let mut records = clone_records(&self.state()?.records);
        records.retain(|record| record.info.id != id);
        self.prepare_records(records, nonces)
    }
    pub fn prepare_password_change(
        &self,
        password: Secret,
        material: RotationMaterial,
    ) -> Result<PreparedWrite, VaultError> {
        validate_password(&password)?;
        let (salt, nonces) = material.into_parts();
        let kek = derive(&password, &salt)?;
        let state = self.state()?;
        let replacement = Unlocked {
            kek,
            salt,
            dek: state.dek.clone(),
            vault_id: state.vault_id,
            records: clone_records(&state.records),
        };
        let encrypted = encrypt(&replacement, nonces)?;
        Ok(PreparedWrite {
            encrypted,
            replacement,
            expected: self.revision.clone(),
        })
    }
    fn state(&self) -> Result<&Unlocked, VaultError> {
        self.unlocked.as_ref().ok_or(VaultError::Locked)
    }
    fn prepare_records(
        &self,
        records: Vec<Record>,
        nonces: SealNonces,
    ) -> Result<PreparedWrite, VaultError> {
        let state = self.state()?;
        let replacement = Unlocked {
            kek: state.kek.clone(),
            dek: state.dek.clone(),
            salt: state.salt,
            vault_id: state.vault_id,
            records,
        };
        let encrypted = encrypt(&replacement, nonces)?;
        Ok(PreparedWrite {
            encrypted,
            replacement,
            expected: self.revision.clone(),
        })
    }

    /// Install a validated candidate only after its adapter publishes it successfully.
    pub fn accept_write(&mut self, prepared: PreparedWrite) {
        self.revision = Some(prepared.encrypted);
        self.unlocked = Some(prepared.replacement);
    }
    pub fn accept_unlock(&mut self, prepared: PreparedUnlock) {
        self.revision = Some(prepared.encrypted);
        self.unlocked = Some(prepared.replacement);
    }
    pub fn device_enrollment(&self) -> Result<DeviceEnrollment, VaultError> {
        let state = self.state()?;
        Ok(DeviceEnrollment {
            binding: VaultBinding {
                vault_id: state.vault_id,
                salt: state.salt,
            },
            key: state.kek.clone(),
        })
    }
}
fn clone_records(records: &[Record]) -> Vec<Record> {
    records
        .iter()
        .map(|record| Record {
            info: record.info.clone(),
            secret: record.secret.clone(),
        })
        .collect()
}
fn validate_password(password: &Secret) -> Result<(), VaultError> {
    if password.expose().chars().count() < 8 || password.expose().len() > 4096 {
        Err(VaultError::Password)
    } else {
        Ok(())
    }
}
fn validate_records(records: &[Record]) -> Result<(), VaultError> {
    if records.len() > MAX_RECORDS
        || records
            .iter()
            .any(|record| record.info.label.len() > 1024 || record.secret.expose().len() > 65536)
    {
        return Err(VaultError::Limit);
    }
    let mut ids = std::collections::HashSet::new();
    if records.iter().any(|record| !ids.insert(record.info.id)) {
        return Err(VaultError::Format);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
