//! Portable authenticated encrypted credential storage.
//!
//! Version 1 uses Argon2id (64 MiB, 3 passes, 4 lanes), a password-wrapped
//! random data key, and XChaCha20-Poly1305. Neither metadata nor secrets leave
//! the encrypted payload. This format needs independent security review.

mod service;
pub use service::{VaultFuture, VaultService};

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
    kek: Zeroizing<[u8; 32]>,
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
fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| VaultError::Randomness)?;
    Ok(bytes)
}
fn derive(password: &Secret, salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    if password.expose().len() > 4096 {
        return Err(VaultError::Password);
    }
    let params =
        Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(32)).map_err(|_| VaultError::Format)?;
    let mut key = Zeroizing::new([0; 32]);
    let mut memory = Zeroizing::new(vec![argon2::Block::default(); MEMORY_KIB as usize]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            password.expose().as_bytes(),
            salt,
            key.as_mut(),
            &mut *memory,
        )
        .map_err(|_| VaultError::Authentication)?;
    Ok(key)
}
fn cipher(key: &[u8; 32]) -> Result<XChaCha20Poly1305, VaultError> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| VaultError::Format)
}

struct Header {
    salt: [u8; 16],
    vault_id: [u8; 16],
    wrap_nonce: [u8; 24],
    payload_nonce: [u8; 24],
}
impl Header {
    fn parse(bytes: &[u8]) -> Result<Self, VaultError> {
        if !(HEADER_SIZE + WRAPPED_SIZE + 16..=MAX_FILE).contains(&bytes.len())
            || &bytes[..8] != MAGIC
            || bytes[8..12] != [0, 1, 1, 19]
        {
            return Err(VaultError::Format);
        }
        let u32_at = |offset| {
            u32::from_be_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("validated header length"),
            )
        };
        // Version 1 deliberately accepts exactly its documented KDF parameters;
        // hostile headers cannot raise memory, iterations or lanes before KDF.
        if u32_at(12) != MEMORY_KIB
            || u32_at(16) != ITERATIONS
            || u32_at(20) != LANES
            || u32_at(104) as usize != bytes.len() - HEADER_SIZE - WRAPPED_SIZE
        {
            return Err(VaultError::Format);
        }
        Ok(Self {
            salt: bytes[24..40].try_into().expect("fixed salt"),
            vault_id: bytes[40..56].try_into().expect("fixed id"),
            wrap_nonce: bytes[56..80].try_into().expect("fixed nonce"),
            payload_nonce: bytes[80..104].try_into().expect("fixed nonce"),
        })
    }
}

fn encode_records(records: &[Record]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_PAYLOAD));
    bytes.extend_from_slice(b"CRD1");
    bytes.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for record in records {
        let binding = Zeroizing::new(
            serde_json::to_vec(&record.info.binding).map_err(|_| VaultError::Format)?,
        );
        if binding.len() > 4096 {
            return Err(VaultError::Limit);
        }
        let id = record.info.id.to_string();
        let fields = [
            id.as_bytes(),
            record.info.label.as_bytes(),
            &binding,
            record.secret.expose().as_bytes(),
        ];
        for field in fields {
            if field.len() + 4 > MAX_PAYLOAD - bytes.len() {
                return Err(VaultError::Limit);
            }
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
            bytes.extend_from_slice(field);
        }
    }
    Ok(bytes)
}
fn decode_records(mut bytes: &[u8]) -> Result<Vec<Record>, VaultError> {
    fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8], VaultError> {
        if length > bytes.len() {
            return Err(VaultError::Format);
        }
        let (value, rest) = bytes.split_at(length);
        *bytes = rest;
        Ok(value)
    }
    fn size(bytes: &mut &[u8]) -> Result<usize, VaultError> {
        Ok(
            u32::from_be_bytes(take(bytes, 4)?.try_into().map_err(|_| VaultError::Format)?)
                as usize,
        )
    }
    fn text<'a>(bytes: &mut &'a [u8], maximum: usize) -> Result<&'a str, VaultError> {
        let length = size(bytes)?;
        if length > maximum {
            return Err(VaultError::Limit);
        }
        std::str::from_utf8(take(bytes, length)?).map_err(|_| VaultError::Format)
    }
    if take(&mut bytes, 4)? != b"CRD1" {
        return Err(VaultError::Format);
    }
    let count = size(&mut bytes)?;
    if count > MAX_RECORDS {
        return Err(VaultError::Limit);
    }
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let id = text(&mut bytes, 32)?
            .parse()
            .map_err(|_| VaultError::Format)?;
        let label = text(&mut bytes, 1024)?.to_owned();
        let binding =
            serde_json::from_str(text(&mut bytes, 4096)?).map_err(|_| VaultError::Format)?;
        let secret = Secret::new(text(&mut bytes, 65536)?.to_owned());
        records.push(Record {
            info: CredentialInfo { id, label, binding },
            secret,
        });
    }
    if !bytes.is_empty() {
        return Err(VaultError::Format);
    }
    Ok(records)
}
fn encrypt(state: &Unlocked) -> Result<Vec<u8>, VaultError> {
    let plaintext = encode_records(&state.records)?;
    let wrap_nonce: [u8; 24] = random()?;
    let payload_nonce: [u8; 24] = random()?;
    let mut bytes = Vec::with_capacity(HEADER_SIZE + WRAPPED_SIZE + plaintext.len() + 16);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&[0, 1, 1, 19]);
    for value in [MEMORY_KIB, ITERATIONS, LANES] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&state.salt);
    bytes.extend_from_slice(&state.vault_id);
    bytes.extend_from_slice(&wrap_nonce);
    bytes.extend_from_slice(&payload_nonce);
    bytes.extend_from_slice(&((plaintext.len() + 16) as u32).to_be_bytes());
    let wrapped = cipher(&state.kek)?
        .encrypt(
            &XNonce::from(wrap_nonce),
            Payload {
                msg: state.dek.as_ref(),
                aad: &bytes,
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    bytes.extend_from_slice(&wrapped);
    let payload = cipher(&state.dek)?
        .encrypt(
            &XNonce::from(payload_nonce),
            Payload {
                msg: &plaintext,
                aad: &bytes,
            },
        )
        .map_err(|_| VaultError::Authentication)?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}
fn read_file(path: &Path) -> Result<Vec<u8>, VaultError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(VaultError::Format);
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE as u64 {
        return Err(VaultError::Limit);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE {
        return Err(VaultError::Limit);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_payload_parser_preserves_secret_bytes_and_rejects_lengths() {
        let secret = "a\"quoted\\password\nwith unicode λ";
        let records = vec![Record {
            info: CredentialInfo {
                id: CredentialId::generate().unwrap(),
                label: "label".into(),
                binding: binding(),
            },
            secret: Secret::new(secret),
        }];
        let encoded = encode_records(&records).unwrap();
        let decoded = decode_records(&encoded).unwrap();
        assert_eq!(decoded[0].secret.expose(), secret);
        for length in [0, 3, 7, encoded.len() - 1] {
            assert!(decode_records(&encoded[..length]).is_err());
        }
        let mut hostile = encoded.to_vec();
        hostile[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_records(&hostile).is_err());
        hostile[..8].copy_from_slice(b"CRD1\xff\xff\xff\xff");
        assert!(decode_records(&hostile).is_err());
    }
    fn password() -> Secret {
        Secret::new("unique test master passphrase")
    }
    fn binding() -> CredentialBinding {
        CredentialBinding::Password {
            target: Target::new("vault-user", "vault-secret-host", 22),
        }
    }
    fn created(path: &Path) -> Vault {
        let mut vault = Vault::new(path);
        vault.create(password()).unwrap();
        vault
    }
    #[test]
    fn wrong_password_tamper_and_lock_do_not_expose_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let mut vault = created(&path);
        let id = vault
            .put(
                None,
                "private metadata".into(),
                binding(),
                Secret::new("very-private-password"),
            )
            .unwrap();
        let original = fs::read(&path).unwrap();
        for plaintext in [
            "very-private-password",
            "private metadata",
            "vault-user",
            "vault-secret-host",
        ] {
            assert!(
                !original
                    .windows(plaintext.len())
                    .any(|bytes| bytes == plaintext.as_bytes())
            );
        }
        assert!(!format!("{vault:?}").contains("very-private-password"));
        vault.lock();
        assert!(matches!(vault.get(id, &binding()), Err(VaultError::Locked)));
        assert!(matches!(
            vault.unlock(Secret::new("wrong password")),
            Err(VaultError::Authentication)
        ));
        assert!(!vault.is_unlocked());
        let mut corrupted = original.clone();
        *corrupted.last_mut().unwrap() ^= 1;
        fs::write(&path, &corrupted).unwrap();
        assert!(matches!(
            vault.unlock(password()),
            Err(VaultError::Authentication)
        ));
        fs::write(&path, original).unwrap();
        vault.unlock(password()).unwrap();
        assert_eq!(
            vault.get(id, &binding()).unwrap().expose(),
            "very-private-password"
        );
    }
    #[test]
    fn hostile_and_truncated_headers_reject_before_kdf() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let _vault = created(&path);
        let original = fs::read(&path).unwrap();
        for length in [0, 7, 24, HEADER_SIZE, HEADER_SIZE + WRAPPED_SIZE + 15] {
            assert!(matches!(
                Header::parse(&original[..length.min(original.len())]),
                Err(VaultError::Format)
            ));
        }
        for offset in [8, 10, 11, 12, 16, 20, 104] {
            let mut bytes = original.clone();
            bytes[offset] ^= 255;
            assert!(matches!(Header::parse(&bytes), Err(VaultError::Format)));
        }
        let mut bytes = original;
        bytes.push(0);
        assert!(matches!(Header::parse(&bytes), Err(VaultError::Format)));
    }
    #[test]
    fn rotation_changes_master_password_and_uses_fresh_nonces() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let mut vault = created(&path);
        let before = fs::read(&path).unwrap();
        let id = vault
            .put(
                None,
                "credential".into(),
                binding(),
                Secret::new("password"),
            )
            .unwrap();
        let after = fs::read(&path).unwrap();
        assert_ne!(&before[56..80], &after[56..80]);
        assert_ne!(&before[80..104], &after[80..104]);
        vault
            .change_password(Secret::new("different master passphrase"))
            .unwrap();
        let rotated = fs::read(&path).unwrap();
        assert_ne!(&after[24..40], &rotated[24..40]);
        vault.lock();
        assert!(vault.unlock(password()).is_err());
        vault
            .unlock(Secret::new("different master passphrase"))
            .unwrap();
        assert_eq!(vault.get(id, &binding()).unwrap().expose(), "password");
    }
    #[test]
    fn stale_writers_and_cancelled_commit_preserve_last_good_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let mut first = created(&path);
        let mut second = Vault::new(&path);
        second.unlock(password()).unwrap();
        first
            .put(None, "one".into(), binding(), Secret::new("one-secret"))
            .unwrap();
        let good = fs::read(&path).unwrap();
        assert!(matches!(
            second.put(None, "two".into(), binding(), Secret::new("two-secret")),
            Err(VaultError::Conflict)
        ));
        assert_eq!(fs::read(&path).unwrap(), good);
        assert!(second.list().unwrap().is_empty());
        first.guard = Some((std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2)), 1));
        assert!(matches!(
            first.put(None, "three".into(), binding(), Secret::new("three-secret")),
            Err(VaultError::Cancelled)
        ));
        assert_eq!(fs::read(&path).unwrap(), good);
        assert_eq!(first.list().unwrap().len(), 1);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        first.guard = None;
        assert!(matches!(
            first.delete(first.list().unwrap()[0].id),
            Err(VaultError::Busy)
        ));
        assert_eq!(fs::read(&path).unwrap(), good);
    }
    #[test]
    fn credential_binding_and_private_permissions_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let mut vault = created(&path);
        let id = vault
            .put(None, "one".into(), binding(), Secret::new("secret"))
            .unwrap();
        let other = CredentialBinding::Password {
            target: Target::new("other", "elsewhere", 22),
        };
        assert!(matches!(vault.get(id, &other), Err(VaultError::Binding)));
        assert!(matches!(
            vault.put(Some(id), "other".into(), other, Secret::new("other")),
            Err(VaultError::Binding)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn worker_lock_invalidates_inflight_unlock_and_autolocks() {
        use futures::executor::block_on;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault");
        let service = VaultService::new(&path, std::time::Duration::from_secs(1)).unwrap();
        block_on(service.create(password())).unwrap();
        assert!(service.is_unlocked());
        service.lock();
        assert!(!service.is_unlocked());
        let unlock = service.unlock(password());
        service.lock();
        assert!(matches!(block_on(unlock), Err(VaultError::Cancelled)));
        assert!(!service.is_unlocked());
        block_on(service.unlock(password())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1400));
        assert!(!service.is_unlocked());
        assert!(matches!(block_on(service.list()), Err(VaultError::Locked)));
    }
}
