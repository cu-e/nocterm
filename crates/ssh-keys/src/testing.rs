//! An in-memory [`VaultSshKeys`] for consumer tests: no vault, no files, no entropy, and a
//! user CA ([`ca`]) that issues OpenSSH certificates for SSH tests.
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

fn lock<T>(state: &Mutex<T>) -> MutexGuard<'_, T> {
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
            let material = KeyMaterial::ed25519_from_seed(&seed, &label)?;
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

mod connected_hosts;
pub use connected_hosts::ScriptedConnectedHosts;

/// An `authorized_keys` file kept in memory, with the same compare-before-swap rule as the
/// adapter that edits a real one.
#[derive(Clone, Default)]
pub struct FakeAuthorizedKeys {
    text: Arc<Mutex<String>>,
    read_only: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeAuthorizedKeys {
    /// A file that starts with `text`.
    pub fn with_text(text: &str) -> Self {
        Self {
            text: Arc::new(Mutex::new(text.to_owned())),
            read_only: Arc::default(),
        }
    }

    /// Replaces the file's text, as an edit on the server would.
    pub fn set_text(&self, text: &str) {
        *self
            .text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = text.to_owned();
    }

    /// Every later replacement fails as if the server refused the write.
    pub fn fail_writes(&self) {
        self.read_only
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The file as it is now.
    pub fn text(&self) -> String {
        self.text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl crate::authorized_keys::AuthorizedKeysFile for FakeAuthorizedKeys {
    fn read(&self) -> BoxFuture<'static, Result<String, crate::authorized_keys::HostAccessError>> {
        Box::pin(std::future::ready(Ok(self.text())))
    }

    fn replace(
        &self,
        previous: String,
        updated: String,
    ) -> BoxFuture<'static, Result<(), crate::authorized_keys::HostAccessError>> {
        let mut text = self
            .text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = if self.read_only.load(std::sync::atomic::Ordering::SeqCst) {
            Err(crate::authorized_keys::HostAccessError::NotText)
        } else if *text == previous {
            *text = updated;
            Ok(())
        } else {
            Err(crate::authorized_keys::HostAccessError::Changed)
        };
        Box::pin(std::future::ready(result))
    }
}

/// Saved connections that are reachable now, each with its in-memory file.
#[derive(Clone, Default)]
pub struct FakeAgentHosts {
    hosts: Arc<Mutex<HashMap<String, crate::authorized_keys::HostAccess>>>,
    ports: Arc<Mutex<HashMap<String, u16>>>,
}

impl FakeAgentHosts {
    /// Makes `connection` reachable as `user@host`, with `file` as its `authorized_keys`.
    pub fn connect(&self, connection: &str, user: &str, host: &str, file: FakeAuthorizedKeys) {
        let access = crate::authorized_keys::HostAccess {
            identity: crate::authorized_keys::HostIdentity {
                user: user.to_owned(),
                host: host.to_owned(),
                host_key: "ssh-ed25519 AAAAfakehostkey".to_owned(),
            },
            file: Arc::new(file),
        };
        self.hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection.to_owned(), access);
    }

    /// Remembers that `connection` was reached on `port`.
    pub fn set_port(&self, connection: &str, port: u16) {
        self.ports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection.to_owned(), port);
    }

    /// Makes `connection` unreachable, as a server that went away.
    pub fn disconnect(&self, connection: &str) {
        self.hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(connection);
    }
}

impl crate::authorized_keys::AgentHosts for FakeAgentHosts {
    fn port(&self, connection: &str) -> Option<u16> {
        self.ports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(connection)
            .copied()
    }
    fn sessions(&self) -> Vec<crate::authorized_keys::HostSession> {
        let hosts = self
            .hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut sessions: Vec<_> = hosts
            .iter()
            .map(|(connection, access)| crate::authorized_keys::HostSession {
                connection: connection.clone(),
                user: access.identity.user.clone(),
                host: access.identity.host.clone(),
            })
            .collect();
        sessions.sort_by(|a, b| a.connection.cmp(&b.connection));
        sessions
    }

    fn access(
        &self,
        connection: &str,
    ) -> Result<crate::authorized_keys::HostAccess, crate::authorized_keys::HostAccessError> {
        self.hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(connection)
            .cloned()
            .ok_or(crate::authorized_keys::HostAccessError::NotConnected)
    }
}
