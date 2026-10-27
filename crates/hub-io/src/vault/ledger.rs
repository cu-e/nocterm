//! The credentials of the vault collection: read, written and deleted as sealed objects.
//!
//! Reads come from the encrypted cache, which every call first tries to bring up to date from
//! the hub; an unreachable hub leaves the cached copy, so credentials stay readable offline.
//! Writes need the hub. A record that does not decode (unknown format or binding) is not
//! listed, cannot be overwritten or deleted, and stays on the hub untouched. A record that
//! does not authenticate is the hub's forgery and is an error, never a silent skip.

use std::collections::BTreeMap;
use std::sync::Arc;

use nocterm_keys::random_bytes;
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::{CredentialBinding, CredentialInfo, VaultError};

use super::codec::{self, Decoded, KIND};
use crate::account::{OpenKeys, Slot};
use crate::cache::{ObjectCopy, VaultCopy};
use crate::classify::unreachable;
use crate::store::{Located, OpError, Store, StoreOps, VAULT_KIND};

/// How often a write retries after another device changed the same record.
const WRITE_ATTEMPTS: usize = 2;

pub(super) struct Ledger {
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
}

/// A failure of a store call, as the vault reports it.
pub(super) fn vault_error(error: OpError) -> VaultError {
    match error {
        OpError::Conflict => VaultError::Conflict,
        OpError::Damaged(_) => VaultError::Authentication,
        OpError::Hub(error) if unreachable(&error) => VaultError::Offline,
        OpError::Hub(error) => VaultError::Io(std::io::Error::other(error)),
    }
}

impl Ledger {
    pub(super) fn new(slot: Arc<Slot>, ops: Arc<dyn StoreOps>) -> Self {
        Self { slot, ops }
    }

    fn keys(&self) -> Result<Arc<OpenKeys>, VaultError> {
        self.slot.open().ok_or(VaultError::Locked)
    }

    /// Brings the cache up to date when the hub is reachable. Only a reachable hub's failure
    /// is an error.
    async fn sync(&self, keys: &Arc<OpenKeys>) -> Result<(), VaultError> {
        let store = Store::new(Arc::clone(&self.ops), Arc::clone(keys));
        match store.find(VAULT_KIND).await {
            Ok(Some(at)) => self.pull(&store, &at).await,
            Ok(None) => Ok(()),
            Err(OpError::Hub(error)) if unreachable(&error) => Ok(()),
            Err(error) => Err(vault_error(error)),
        }
    }

    /// Reads the changes after the cached position and writes the cache through.
    async fn pull(&self, store: &Store, at: &Located) -> Result<(), VaultError> {
        let mut copy = self.slot.read_cache(|cache| {
            cache
                .and_then(|cache| cache.vault.clone())
                .filter(|vault| vault.collection_id == at.id())
        });
        let mut vault = copy.take().unwrap_or_else(|| VaultCopy {
            collection_id: at.id().to_owned(),
            key_envelope: at.envelope().to_vec(),
            cursor: 0,
            objects: BTreeMap::new(),
        });
        vault.key_envelope = at.envelope().to_vec();
        loop {
            let batch = store.changes(at, vault.cursor).await.map_err(vault_error)?;
            for object in batch.changes {
                vault.cursor = vault.cursor.max(object.stream_version);
                vault.objects.insert(
                    object.object_id,
                    ObjectCopy {
                        version: object.version,
                        kind: object.kind,
                        ciphertext: object.ciphertext,
                        deleted: object.deleted,
                    },
                );
            }
            if !batch.more {
                vault.cursor = vault.cursor.max(batch.head);
                break;
            }
        }
        if let Err(error) = self.slot.update_cache(|cache| cache.vault = Some(vault)) {
            tracing::warn!(%error, "the vault cache cannot be written");
        }
        Ok(())
    }

    /// Opens one cached object: `Ok(None)` when it is a tombstone or another kind, and an
    /// error when it does not authenticate.
    fn open(
        keys: &OpenKeys,
        vault: &VaultCopy,
        object_id: &str,
        object: &ObjectCopy,
    ) -> Result<Option<Decoded>, VaultError> {
        if object.deleted || object.kind != KIND {
            return Ok(None);
        }
        let key = keys
            .collection(&vault.collection_id, &vault.key_envelope)
            .map_err(|_| VaultError::Authentication)?;
        let plain = key
            .open_object(&vault.collection_id, object_id, KIND, &object.ciphertext)
            .map_err(|_| VaultError::Authentication)?;
        Ok(Some(codec::decode(&plain)))
    }

    fn cached(&self) -> Option<VaultCopy> {
        self.slot
            .read_cache(|cache| cache.and_then(|cache| cache.vault.clone()))
    }

    pub(super) async fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        let keys = self.keys()?;
        self.sync(&keys).await?;
        let Some(vault) = self.cached() else {
            return Ok(Vec::new());
        };
        let mut listed = Vec::new();
        for (object_id, object) in &vault.objects {
            if let Some(Decoded::Known(record)) = Self::open(&keys, &vault, object_id, object)?
                && let Ok(id) = object_id.parse::<CredentialId>()
            {
                listed.push(CredentialInfo {
                    id,
                    label: record.label,
                    binding: record.binding,
                    group: None,
                });
            }
        }
        Ok(listed)
    }

    pub(super) async fn get(
        &self,
        id: CredentialId,
        binding: CredentialBinding,
    ) -> Result<Secret, VaultError> {
        let keys = self.keys()?;
        self.sync(&keys).await?;
        let vault = self.cached().ok_or(VaultError::NotFound)?;
        let object_id = id.to_string();
        let object = vault.objects.get(&object_id).ok_or(VaultError::NotFound)?;
        match Self::open(&keys, &vault, &object_id, object)? {
            Some(Decoded::Known(record)) if record.binding == binding => Ok(record.secret),
            Some(Decoded::Known(_)) => Err(VaultError::Binding),
            Some(Decoded::Unrecognized) | None => Err(VaultError::NotFound),
        }
    }

    pub(super) async fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> Result<CredentialId, VaultError> {
        let keys = self.keys()?;
        let store = Store::new(Arc::clone(&self.ops), Arc::clone(&keys));
        let id = match id {
            Some(id) => id,
            None => {
                CredentialId::from_bytes(random_bytes::<16>().map_err(|_| VaultError::Randomness)?)
            }
        };
        let object_id = id.to_string();
        let plaintext = codec::encode(&label, &binding, &secret).map_err(|_| VaultError::Format)?;
        for attempt in 1..=WRITE_ATTEMPTS {
            let at = store.ensure(VAULT_KIND).await.map_err(vault_error)?;
            self.pull(&store, &at).await?;
            let expected = self.expected(&keys, &object_id, true)?;
            match store.put(&at, KIND, &object_id, &plaintext, expected).await {
                Ok(_) => {
                    self.pull(&store, &at).await?;
                    return Ok(id);
                }
                Err(OpError::Conflict) if attempt < WRITE_ATTEMPTS => {}
                Err(error) => return Err(vault_error(error)),
            }
        }
        Err(VaultError::Conflict)
    }

    pub(super) async fn delete(&self, id: CredentialId) -> Result<(), VaultError> {
        let keys = self.keys()?;
        let store = Store::new(Arc::clone(&self.ops), Arc::clone(&keys));
        let object_id = id.to_string();
        for attempt in 1..=WRITE_ATTEMPTS {
            let at = store
                .find(VAULT_KIND)
                .await
                .map_err(vault_error)?
                .ok_or(VaultError::NotFound)?;
            self.pull(&store, &at).await?;
            let expected = self.expected(&keys, &object_id, false)?;
            match store.remove(&at, &object_id, expected).await {
                Ok(_) => return self.pull(&store, &at).await,
                Err(OpError::Conflict) if attempt < WRITE_ATTEMPTS => {}
                Err(error) => return Err(vault_error(error)),
            }
        }
        Err(VaultError::Conflict)
    }

    /// The version a write must expect for `object_id`. A record that is not ours to change
    /// (it does not decode) is [`VaultError::Format`] when writing and not found when deleting.
    fn expected(
        &self,
        keys: &OpenKeys,
        object_id: &str,
        creating: bool,
    ) -> Result<u64, VaultError> {
        let missing = if creating {
            Ok(0)
        } else {
            Err(VaultError::NotFound)
        };
        let Some(vault) = self.cached() else {
            return missing;
        };
        let Some(object) = vault.objects.get(object_id) else {
            return missing;
        };
        if object.deleted {
            return if creating {
                Ok(object.version)
            } else {
                Err(VaultError::NotFound)
            };
        }
        match Self::open(keys, &vault, object_id, object)? {
            Some(Decoded::Known(_)) => Ok(object.version),
            _ if creating => Err(VaultError::Format),
            _ => Err(VaultError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests;
