//! The narrow port the connection editor uses to pick, create and import vault keys.
use crate::{PublicKeyText, VaultSshKeysError};
use futures::future::BoxFuture;
use nocterm_session::{CredentialId, Secret};
use std::path::PathBuf;

/// A key kept in the vault, as the editor lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredKey {
    pub credential: CredentialId,
    pub label: String,
    pub public_key: PublicKeyText,
    pub fingerprint: String,
}

/// Where the text of a key to import comes from.
#[derive(Clone, Debug)]
pub enum KeySource {
    /// A private key file; the implementation reads it within bounds.
    File(PathBuf),
    /// Key text already in memory.
    Text(Secret),
}

/// The vault keys of whichever vault is selected now.
pub trait VaultSshKeys: Send + Sync {
    /// The keys stored in the vault, oldest first.
    fn list(&self) -> BoxFuture<'static, Result<Vec<StoredKey>, VaultSshKeysError>>;
    /// Creates an Ed25519 key and stores it under `label`.
    fn generate(&self, label: String) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>>;
    /// Stores the key from `source`, removing its `passphrase`, under `label`.
    fn import(
        &self,
        label: String,
        source: KeySource,
        passphrase: Option<Secret>,
    ) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>>;
}

/// The port of a window composed without a vault: every call says so.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoVaultSshKeys;

impl VaultSshKeys for NoVaultSshKeys {
    fn list(&self) -> BoxFuture<'static, Result<Vec<StoredKey>, VaultSshKeysError>> {
        Box::pin(std::future::ready(Err(VaultSshKeysError::Unavailable)))
    }
    fn generate(&self, _: String) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        Box::pin(std::future::ready(Err(VaultSshKeysError::Unavailable)))
    }
    fn import(
        &self,
        _: String,
        _: KeySource,
        _: Option<Secret>,
    ) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        Box::pin(std::future::ready(Err(VaultSshKeysError::Unavailable)))
    }
}
