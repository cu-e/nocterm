//! One private persistent runner identity, with no account private key or provider secret.
mod file;
use file::IdentityFile;
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_core::SharedCause;
use nocterm_keys::DeviceKey;
use nocterm_keys::cloud::{RunnerEncryptionKey, RunnerSigningKey};
use std::{
    fmt, io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum CloudRunnerIdentityError {
    #[error("the private runner directory could not be prepared")]
    Directory(#[source] io::Error),
    #[error("the runner identity is already owned or its lock is unavailable")]
    Lock(#[source] io::Error),
    #[error("the runner identity file cannot be opened privately")]
    Private(#[source] nocterm_core::fs::PrivateFileError),
    #[error("the runner identity could not be published without replacement")]
    Publication(#[source] nocterm_core::fs::FileError),
    #[error("the existing runner identity could not be read")]
    Read(#[source] io::Error),
    #[error("the existing runner identity is malformed; explicit recovery is required")]
    Format,
    #[error("the runner identity belongs to another original account or runner")]
    Binding,
    #[error("the identity exists but private durable readiness is unconfirmed")]
    StoredButUnavailable(#[source] SharedCause),
    #[error("the operating system could not generate runner key material")]
    KeyMaterial(#[source] SharedCause),
    #[error("the original runner identity is retired or unavailable")]
    Unavailable,
}

/// Descriptive public identity, not a trusted descriptor or enrollment receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerPublicIdentity {
    binding: JournalCatalogBinding,
    encryption: [u8; 32],
    signing: [u8; 32],
    proof: [u8; 32],
}
impl RunnerPublicIdentity {
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption
    }
    pub fn signing_key(&self) -> &[u8; 32] {
        &self.signing
    }
    pub fn proof_key(&self) -> &[u8; 32] {
        &self.proof
    }
}

pub(super) struct IdentityKeys {
    encryption: RunnerEncryptionKey,
    signing: RunnerSigningKey,
    proof: DeviceKey,
}
impl IdentityKeys {
    fn generate() -> Result<Self, CloudRunnerIdentityError> {
        Ok(Self {
            encryption: RunnerEncryptionKey::generate().map_err(|source| {
                CloudRunnerIdentityError::KeyMaterial(SharedCause::new(source))
            })?,
            signing: RunnerSigningKey::generate().map_err(|source| {
                CloudRunnerIdentityError::KeyMaterial(SharedCause::new(source))
            })?,
            // A separate key authenticates Cloud-only tokens; it never becomes
            // a device session or a transcript signing capability.
            proof: DeviceKey::generate().map_err(|source| {
                CloudRunnerIdentityError::KeyMaterial(SharedCause::new(source))
            })?,
        })
    }
    pub(super) fn encryption(&self) -> &RunnerEncryptionKey {
        &self.encryption
    }
    pub(super) fn signing(&self) -> &RunnerSigningKey {
        &self.signing
    }
    pub(super) fn proof(&self) -> &DeviceKey {
        &self.proof
    }
}

/// Owns keys and a cooperative process-lifetime lock in a trusted private parent.
/// Does not enroll, fetch credentials, replace identities, or export a signer/seed.
/// Call creation/opening on a blocking startup worker, before network publication.
pub struct CloudRunnerIdentity {
    binding: JournalCatalogBinding,
    _files: IdentityFile,
    keys: Mutex<Option<IdentityKeys>>,
    retired: AtomicBool,
}
impl CloudRunnerIdentity {
    /// Explicit first installation. Existing identities, including malformed
    /// files or links, are never overwritten. Descriptive binding is supplied
    /// by trusted setup; the real published public keys must still be pinned.
    pub fn create_new(
        root: &Path,
        binding: JournalCatalogBinding,
    ) -> Result<Self, CloudRunnerIdentityError> {
        let mut files = IdentityFile::acquire(root)?;
        let keys = IdentityKeys::generate()?;
        files.create(&binding, &keys)?;
        Ok(Self {
            binding,
            _files: files,
            keys: Mutex::new(Some(keys)),
            retired: AtomicBool::new(false),
        })
    }
    /// Startup/recovery opens exactly the original binding and seeds. Missing,
    /// invalid or nonprivate data is a refusal, never a new identity fallback.
    pub fn open_existing(
        root: &Path,
        expected: JournalCatalogBinding,
    ) -> Result<Self, CloudRunnerIdentityError> {
        let mut files = IdentityFile::acquire(root)?;
        let keys = files.open(&expected)?;
        Ok(Self {
            binding: expected,
            _files: files,
            keys: Mutex::new(Some(keys)),
            retired: AtomicBool::new(false),
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn public_identity(&self) -> Result<RunnerPublicIdentity, CloudRunnerIdentityError> {
        self.with_keys(|keys| RunnerPublicIdentity {
            binding: self.binding.clone(),
            encryption: keys.encryption.public_key().to_bytes(),
            signing: keys.signing.public_key(),
            proof: keys.proof.public_key(),
        })
    }
    /// Ends this owner's capabilities without deleting/resetting persistent keys.
    /// The app separately retires the corresponding Cloud SDK session and tasks.
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        // Recovering a poisoned guard here only destroys secrets; it never reads
        // or republishes state whose invariant might have been interrupted.
        self.keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    pub(super) fn with_keys<T>(
        &self,
        action: impl FnOnce(&IdentityKeys) -> T,
    ) -> Result<T, CloudRunnerIdentityError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(CloudRunnerIdentityError::Unavailable);
        }
        let keys = self
            .keys
            .lock()
            .map_err(|_| CloudRunnerIdentityError::Unavailable)?;
        if self.retired.load(Ordering::Acquire) {
            return Err(CloudRunnerIdentityError::Unavailable);
        }
        let result = action(keys.as_ref().ok_or(CloudRunnerIdentityError::Unavailable)?);
        if self.retired.load(Ordering::Acquire) {
            return Err(CloudRunnerIdentityError::Unavailable);
        }
        Ok(result)
    }
}
impl fmt::Debug for CloudRunnerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudRunnerIdentity")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
