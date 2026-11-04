//! Fixed native160-byte identity record. These bytes are never a public wire format.
use super::{CloudRunnerIdentityError as Error, IdentityKeys};
use crate::cloud::private_snapshot::{PrivateSnapshot, SnapshotError};
use nocterm_cloud_model::journal::JournalCatalogBinding;
use nocterm_keys::DeviceKey;
use nocterm_keys::cloud::{RunnerEncryptionKey, RunnerSigningKey};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read as _, path::Path};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"NCRID001";
const BYTES: usize = 160;

pub(super) struct IdentityFile {
    file: PrivateSnapshot,
    identity: Option<File>,
}
impl IdentityFile {
    pub(super) fn acquire(root: &Path) -> Result<Self, Error> {
        Ok(Self {
            file: PrivateSnapshot::acquire(root, "runner-identity.lock", "runner-identity.v1")
                .map_err(snapshot)?,
            identity: None,
        })
    }
    pub(super) fn create(
        &mut self,
        binding: &JournalCatalogBinding,
        keys: &IdentityKeys,
    ) -> Result<(), Error> {
        let mut bytes = Zeroizing::new([0u8; BYTES]);
        bytes[..8].copy_from_slice(MAGIC);
        bytes[40..56].copy_from_slice(binding.runner().as_bytes());
        bytes[56..64].copy_from_slice(&binding.epoch().get().to_be_bytes());
        bytes[64..96].copy_from_slice(keys.encryption().seed().as_ref());
        bytes[96..128].copy_from_slice(keys.signing().seed().as_ref());
        bytes[128..160].copy_from_slice(keys.proof().seed().as_ref());
        let digest = identity_digest(binding, &bytes[40..160]);
        bytes[8..40].copy_from_slice(&digest);
        let file = self.file.publish_new(bytes.as_ref()).map_err(snapshot)?;
        // From here failure means the identity already exists, never rollback.
        self.identity = Some(file);
        self.confirm_ready()
    }
    pub(super) fn open(&mut self, binding: &JournalCatalogBinding) -> Result<IdentityKeys, Error> {
        let mut file = self.file.open().map_err(Error::Private)?;
        let mut bytes = Zeroizing::new([0u8; BYTES + 1]);
        let mut used = 0;
        loop {
            if used == bytes.len() {
                return Err(Error::Format);
            }
            let read = file.read(&mut bytes[used..]).map_err(Error::Read)?;
            if read == 0 {
                break;
            }
            used += read;
        }
        if used != BYTES || &bytes[..8] != MAGIC {
            return Err(Error::Format);
        }
        if bytes[8..40] != identity_digest(binding, &bytes[40..160])
            || &bytes[40..56] != binding.runner().as_bytes()
            || bytes[56..64] != binding.epoch().get().to_be_bytes()
        {
            return Err(Error::Binding);
        }
        let encryption = bytes[64..96].try_into().map_err(|_| Error::Format)?;
        let signing = bytes[96..128].try_into().map_err(|_| Error::Format)?;
        let keys = IdentityKeys {
            encryption: RunnerEncryptionKey::from_seed(encryption),
            signing: RunnerSigningKey::from_seed(signing),
            proof: DeviceKey::from_seed(bytes[128..160].try_into().map_err(|_| Error::Format)?),
        };
        self.identity = Some(file);
        self.confirm_ready()?;
        Ok(keys)
    }
    fn confirm_ready(&self) -> Result<(), Error> {
        let identity = self.identity.as_ref().ok_or(Error::Unavailable)?;
        self.file.confirm(identity).map_err(snapshot)
    }
}

fn snapshot(error: SnapshotError) -> Error {
    match error {
        SnapshotError::Directory(source) => Error::Directory(source),
        SnapshotError::Lock(source) => Error::Lock(source),
        SnapshotError::Private(source) => Error::Private(source),
        SnapshotError::Publication(source) => Error::Publication(source),
        SnapshotError::Stored(source) => Error::StoredButUnavailable(source),
    }
}

/// Native-only source/material checksum, including every original seed bit.
/// Public-key equality alone misses X25519 clamped bits that still contribute
/// to the SDK's native metadata derivation. The I/O buffer remains zeroizing.
/// This is not authentication against a trusted owner who can rewrite the file,
/// nor a second implementation of a signed public Cloud coordinate.
fn identity_digest(binding: &JournalCatalogBinding, material: &[u8]) -> [u8; 32] {
    let scope = binding.scope();
    let authority = scope.hub().authority();
    let account_key = scope.account().key();
    let mut digest = Sha256::new();
    digest.update(b"nocterm-native-runner-identity-v1");
    for field in [
        scope.hub().instance().as_str().as_bytes(),
        authority.as_bytes(),
        scope.account().id().as_str().as_bytes(),
        account_key.as_bytes(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.update(material);
    // Flush raw material from the streaming hash's retained input block before
    // finalization. This fixed padding is part of this private format's v1.
    digest.update([0u8; 64]);
    digest.finalize().into()
}
