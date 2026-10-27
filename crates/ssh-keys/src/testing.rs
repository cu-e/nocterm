//! An in-memory [`VaultSshKeys`] for consumer tests: no vault, no files, no entropy.
use crate::{KeyImportError, KeyMaterial, KeySource, StoredKey, VaultSshKeys, VaultSshKeysError};
use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;
use nocterm_session::{CredentialId, Secret};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

struct Entry {
    key: StoredKey,
    secret: Secret,
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    files: HashMap<PathBuf, Secret>,
    locked: bool,
    next: u128,
}

/// Keys kept in memory. Generation derives each seed from a counter, so a test
/// sees the same keys on every run.
#[derive(Clone, Default)]
pub struct MemoryVaultSshKeys {
    state: Arc<Mutex<State>>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, thiserror::Error)]
#[error("no such file in the fake file system")]
struct MissingFile;

impl MemoryVaultSshKeys {
    /// Makes `path` readable by [`KeySource::File`].
    pub fn with_file(self, path: impl AsRef<Path>, text: Secret) -> Self {
        lock(&self.state)
            .files
            .insert(path.as_ref().to_owned(), text);
        self
    }

    /// Locks or unlocks the fake vault; every call fails with `Locked` while locked.
    pub fn set_locked(&self, locked: bool) {
        lock(&self.state).locked = locked;
    }

    /// The stored OpenSSH text of a key, as the vault would hold it.
    pub fn secret_of(&self, credential: CredentialId) -> Option<Secret> {
        lock(&self.state)
            .entries
            .iter()
            .find(|entry| entry.key.credential == credential)
            .map(|entry| entry.secret.clone())
    }

    fn store(
        state: &Mutex<State>,
        label: String,
        material: &KeyMaterial,
    ) -> Result<StoredKey, VaultSshKeysError> {
        let mut state = lock(state);
        state.next += 1;
        let key = StoredKey {
            credential: CredentialId::from_bytes(state.next.to_be_bytes()),
            label,
            public_key: material.public_key(),
            fingerprint: material.fingerprint().to_owned(),
        };
        state.entries.push(Entry {
            key: key.clone(),
            secret: material.to_secret(),
        });
        Ok(key)
    }

    fn unlocked(state: &Mutex<State>) -> Result<(), VaultSshKeysError> {
        if lock(state).locked {
            Err(VaultSshKeysError::Locked)
        } else {
            Ok(())
        }
    }
}

impl VaultSshKeys for MemoryVaultSshKeys {
    fn list(&self) -> BoxFuture<'static, Result<Vec<StoredKey>, VaultSshKeysError>> {
        let state = self.state.clone();
        Box::pin(async move {
            Self::unlocked(&state)?;
            Ok(lock(&state)
                .entries
                .iter()
                .map(|entry| entry.key.clone())
                .collect())
        })
    }

    fn generate(&self, label: String) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        let state = self.state.clone();
        Box::pin(async move {
            Self::unlocked(&state)?;
            let mut seed = [0; 32];
            seed[16..].copy_from_slice(&(lock(&state).next + 1).to_be_bytes());
            let material = KeyMaterial::ed25519_from_seed(seed, &label)?;
            Self::store(&state, label, &material)
        })
    }

    fn import(
        &self,
        label: String,
        source: KeySource,
        passphrase: Option<Secret>,
    ) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        let state = self.state.clone();
        Box::pin(async move {
            Self::unlocked(&state)?;
            let text = match source {
                KeySource::Text(text) => text,
                KeySource::File(path) => lock(&state)
                    .files
                    .get(&path)
                    .cloned()
                    .ok_or_else(|| VaultSshKeysError::Read(SharedCause::new(MissingFile)))?,
            };
            let material = KeyMaterial::import(&text, passphrase.as_ref())
                .map_err(|error: KeyImportError| VaultSshKeysError::Key(error))?;
            Self::store(&state, label, &material)
        })
    }
}
