//! SSH keys of the vault: the connection editor's [`VaultSshKeys`] over the
//! vault, with entropy and key files supplied by the SSH adapter.
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_core::error::SharedCause;
use nocterm_session::Secret;
use nocterm_ssh::keys::{GenerateKeyError, ReadKeyError, generate_ed25519, read_private_key};
use nocterm_ssh_keys::{
    KeyMaterial, KeySource, PublicKeyText, StoredKey, VaultSshKeys, VaultSshKeysError,
};
use nocterm_vault::{CredentialBinding, CredentialInfo, VaultAccess, VaultError};
use std::sync::Arc;

/// The keys of one vault.
pub(crate) struct VaultKeys {
    vault: Arc<dyn VaultAccess>,
}

impl VaultKeys {
    pub(crate) fn new(vault: Arc<dyn VaultAccess>) -> Self {
        Self { vault }
    }
}

fn vault_error(error: VaultError) -> VaultSshKeysError {
    match error {
        VaultError::Locked => VaultSshKeysError::Locked,
        other => VaultSshKeysError::Vault(SharedCause::new(other)),
    }
}

/// A listed vault record that holds a readable key; any other is not a key.
fn stored_key(info: CredentialInfo) -> Option<StoredKey> {
    let CredentialBinding::PrivateKey { public_key } = info.binding else {
        return None;
    };
    let public_key = PublicKeyText::parse(&public_key).ok()?;
    Some(StoredKey {
        credential: info.id,
        label: info.label,
        fingerprint: public_key.fingerprint().to_owned(),
        public_key,
    })
}

/// Keeps `material` in the vault under `label`, bound to its public half.
fn keep(
    vault: &dyn VaultAccess,
    label: String,
    material: &KeyMaterial,
) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
    let public_key = material.public_key();
    let stored = vault.put(
        None,
        label.clone(),
        CredentialBinding::PrivateKey {
            public_key: public_key.as_str().to_owned(),
        },
        material.to_secret(),
    );
    let fingerprint = material.fingerprint().to_owned();
    async move {
        let credential = stored.await.map_err(vault_error)?;
        Ok(StoredKey {
            credential,
            label,
            public_key,
            fingerprint,
        })
    }
    .boxed()
}

/// Runs blocking work (a bounded file read, a slow key derivation) on its own
/// thread; `None` means the thread died before answering.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> {
    let (send, receive) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = send.send(work());
    });
    async move { receive.await.ok() }
}

/// The key text of `source`, read within bounds.
async fn key_text(source: KeySource) -> Result<Secret, VaultSshKeysError> {
    match source {
        KeySource::Text(text) => Ok(text),
        KeySource::File(path) => match off_thread(move || read_private_key(&path)).await {
            Some(Ok(text)) => Ok(text),
            Some(Err(error)) => Err(VaultSshKeysError::Read(SharedCause::new(error))),
            None => Err(VaultSshKeysError::Read(SharedCause::new(ReadKeyError::Io(
                std::io::Error::other("the key file reader stopped"),
            )))),
        },
    }
}

impl VaultSshKeys for VaultKeys {
    fn list(&self) -> BoxFuture<'static, Result<Vec<StoredKey>, VaultSshKeysError>> {
        let listed = self.vault.list();
        async move {
            Ok(listed
                .await
                .map_err(vault_error)?
                .into_iter()
                .filter_map(stored_key)
                .collect())
        }
        .boxed()
    }

    fn generate(&self, label: String) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        if !self.vault.is_unlocked() {
            return futures::future::ready(Err(VaultSshKeysError::Locked)).boxed();
        }
        match generate_ed25519(&label) {
            Ok(material) => keep(self.vault.as_ref(), label, &material),
            Err(GenerateKeyError::Entropy(error)) => futures::future::ready(Err(
                VaultSshKeysError::Entropy(SharedCause::new(GenerateKeyError::Entropy(error))),
            ))
            .boxed(),
            Err(GenerateKeyError::Key(error)) => {
                futures::future::ready(Err(VaultSshKeysError::Key(error))).boxed()
            }
        }
    }

    fn import(
        &self,
        label: String,
        source: KeySource,
        passphrase: Option<Secret>,
    ) -> BoxFuture<'static, Result<StoredKey, VaultSshKeysError>> {
        if !self.vault.is_unlocked() {
            return futures::future::ready(Err(VaultSshKeysError::Locked)).boxed();
        }
        let vault = self.vault.clone();
        async move {
            let text = key_text(source).await?;
            let material = off_thread(move || KeyMaterial::import(&text, passphrase.as_ref()))
                .await
                .ok_or_else(|| {
                    VaultSshKeysError::Read(SharedCause::new(ReadKeyError::Io(
                        std::io::Error::other("the key reader stopped"),
                    )))
                })??;
            keep(vault.as_ref(), label, &material).await
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests;
