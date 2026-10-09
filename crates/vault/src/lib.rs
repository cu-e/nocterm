//! Portable authenticated encrypted credential storage.
//!
//! Version 1 uses Argon2id (64 MiB, 3 passes, 4 lanes), a password-wrapped
//! random data key, and XChaCha20-Poly1305. Neither metadata nor secrets leave
//! the encrypted payload. This format needs independent security review.

mod device_unlock;
mod format;
mod service;
pub use device_unlock::{
    DeviceAvailability, DeviceCancellation, DeviceCapability, DeviceRegistrationState,
    DeviceUnlockError, DeviceUnlockProvider, VaultBinding, VaultKey,
};
pub use service::{VaultFuture, VaultService, VaultStatus};

use format::{Header, cipher, decode_records, derive, encrypt, random, read_file};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use fs2::FileExt as _;
use nocterm_session::{CredentialId, Secret, Target};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};
use zeroize::{Zeroize as _, Zeroizing};

const MAGIC: &[u8; 8] = b"NOCVAULT";
const HEADER_SIZE: usize = 108;
const WRAPPED_SIZE: usize = 48;
const MAX_PAYLOAD: usize = 1_048_576;
const MAX_FILE: usize = HEADER_SIZE + WRAPPED_SIZE + MAX_PAYLOAD + 16;
const MEMORY_KIB: u32 = 65536;
const ITERATIONS: u32 = 3;
const LANES: u32 = 4;
const MAX_RECORDS: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("The vault is locked.")]
    Locked,
    #[error("The vault password is incorrect or the vault was modified.")]
    Authentication,
    #[error("The vault format is invalid or unsupported.")]
    Format,
    #[error("The vault changed in another process. Lock and unlock it again before saving.")]
    Conflict,
    #[error("Another vault operation is in progress. Try again.")]
    Busy,
    #[error("The vault already exists.")]
    AlreadyExists,
    #[error("The credential does not match this authentication request.")]
    Binding,
    #[error("The credential was not found.")]
    NotFound,
    #[error("The vault or credential exceeds the supported size limit.")]
    Limit,
    #[error("Use a master password of at least 8 characters and at most 4096 bytes.")]
    Password,
    #[error("Operating system randomness is unavailable.")]
    Randomness,
    #[error("The operation was cancelled because the vault was locked.")]
    Cancelled,
    #[error("Vault file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Device(#[from] DeviceUnlockError),
}

/// The specific request a stored secret may answer. Interactive/MFA answers
/// have no representation here and can never be saved by this API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialBinding {
    Password { target: Target },
    KeyPassphrase { path: PathBuf },
}

#[derive(Clone, Debug)]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub label: String,
    pub binding: CredentialBinding,
}

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

/// Synchronous backend. Use VaultService from UI code: KDF and file operations
/// run on its single bounded worker, never on the foreground executor.
pub struct Vault {
    path: PathBuf,
    revision: Option<Vec<u8>>,
    unlocked: Option<Unlocked>,
    device: Option<std::sync::Arc<dyn DeviceUnlockProvider>>,
    guard: Option<(std::sync::Arc<std::sync::atomic::AtomicU64>, u64)>,
}
impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("unlocked", &self.is_unlocked())
            .finish_non_exhaustive()
    }
}

impl Vault {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            revision: None,
            unlocked: None,
            device: None,
            guard: None,
        }
    }
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }
    pub fn lock(&mut self) {
        self.unlocked = None;
        self.revision = None;
    }

    pub fn create(&mut self, password: Secret) -> Result<(), VaultError> {
        validate_password(&password)?;
        if self.exists() {
            return Err(VaultError::AlreadyExists);
        }
        let salt = random()?;
        let state = Unlocked {
            kek: derive(&password, &salt)?,
            dek: Zeroizing::new(random()?),
            salt,
            vault_id: random()?,
            records: Vec::new(),
        };
        let encrypted = encrypt(&state)?;
        self.persist(&encrypted, None)?;
        self.revision = Some(encrypted);
        self.unlocked = Some(state);
        Ok(())
    }

    pub fn unlock(&mut self, password: Secret) -> Result<(), VaultError> {
        // Failed unlock attempts do not leave an older unlocked state usable.
        self.lock();
        let encrypted = read_file(&self.path)?;
        let header = Header::parse(&encrypted)?;
        let kek = derive(&password, &header.salt)?;
        self.unlock_encrypted(encrypted, header, kek)
    }

    fn unlock_encrypted(
        &mut self,
        encrypted: Vec<u8>,
        header: Header,
        kek: VaultKey,
    ) -> Result<(), VaultError> {
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
        self.unlocked = Some(Unlocked {
            kek,
            dek,
            salt: header.salt,
            vault_id: header.vault_id,
            records,
        });
        self.revision = Some(encrypted);
        Ok(())
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
    pub fn put(
        &mut self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> Result<CredentialId, VaultError> {
        let id = match id {
            Some(id) => id,
            None => CredentialId::generate().map_err(|_| VaultError::Randomness)?,
        };
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
        self.write_records(records)?;
        Ok(id)
    }
    pub fn delete(&mut self, id: CredentialId) -> Result<(), VaultError> {
        let mut records = clone_records(&self.state()?.records);
        records.retain(|record| record.info.id != id);
        self.write_records(records)
    }
    pub fn change_password(&mut self, password: Secret) -> Result<(), VaultError> {
        validate_password(&password)?;
        let salt = random()?;
        let kek = derive(&password, &salt)?;
        let state = self.state()?;
        let replacement = Unlocked {
            kek,
            salt,
            dek: state.dek.clone(),
            vault_id: state.vault_id,
            records: clone_records(&state.records),
        };
        let encrypted = encrypt(&replacement)?;
        self.persist(&encrypted, self.revision.as_deref())?;
        self.revision = Some(encrypted);
        self.unlocked = Some(replacement);
        self.revoke_device_after_rotation();
        Ok(())
    }
    fn state(&self) -> Result<&Unlocked, VaultError> {
        self.unlocked.as_ref().ok_or(VaultError::Locked)
    }
    fn write_records(&mut self, records: Vec<Record>) -> Result<(), VaultError> {
        let state = self.state()?;
        let replacement = Unlocked {
            kek: state.kek.clone(),
            dek: state.dek.clone(),
            salt: state.salt,
            vault_id: state.vault_id,
            records,
        };
        let encrypted = encrypt(&replacement)?;
        self.persist(&encrypted, self.revision.as_deref())?;
        self.revision = Some(encrypted);
        self.unlocked = Some(replacement);
        Ok(())
    }
    fn persist(&self, encrypted: &[u8], expected: Option<&[u8]>) -> Result<(), VaultError> {
        let directory = self.path.parent().ok_or(VaultError::Format)?;
        fs::create_dir_all(directory)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(self.path.with_extension("lock"))?;
        lock.try_lock_exclusive().map_err(|_| VaultError::Busy)?;
        let current = match read_file(&self.path) {
            Ok(current) => Some(current),
            Err(VaultError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        match (expected, current.as_deref()) {
            (None, None) => {}
            (None, Some(_)) => return Err(VaultError::AlreadyExists),
            (Some(expected), Some(current)) if expected == current => {}
            _ => return Err(VaultError::Conflict),
        }
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temporary.write_all(encrypted)?;
        temporary.as_file().sync_all()?;
        if self.guard.as_ref().is_some_and(|(epoch, expected)| {
            epoch.load(std::sync::atomic::Ordering::SeqCst) != *expected
        }) {
            return Err(VaultError::Cancelled);
        }
        temporary
            .persist(&self.path)
            .map_err(|error| VaultError::Io(error.error))?;
        // A parent-directory fsync failure happens after commit: do not return a
        // precommit error that would make in-memory revision disagree with disk.
        #[cfg(unix)]
        {
            if let Ok(directory) = File::open(directory) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
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
