//! The credentials of the vault collection: read, written and deleted as sealed objects.
//!
//! Reads come from the encrypted cache, which every call first tries to bring up to date from
//! the hub; an unreachable hub leaves the cached copy, so credentials stay readable offline.
//! Writes need the hub. A record that does not decode (unknown format or binding) is not
//! listed, cannot be overwritten or deleted, and stays on the hub untouched. A record that
//! does not authenticate is the hub's forgery and is an error, never a silent skip.
//!
//! The credentials of the account's organization groups are read through [`Groups`] and
//! routed by the collection that holds them: an id that a group holds is written and deleted
//! in that group, with that group's key, and never in the personal collection.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use nocterm_hubs_model::HubError;
use nocterm_keys::random_bytes;
use nocterm_session::{CredentialId, Secret};
use nocterm_vault::{CredentialBinding, CredentialInfo, VaultError};

use super::codec::{self, Decoded, KIND};
use super::groups::{Groups, Held, Owner};
use crate::account::{OpenKeys, Slot, source::CapturedAccount};
mod access;
use super::snapshot_budget::{SnapshotBudget, merge_vault};
use crate::cache::{ObjectCopy, VaultCopy};
use crate::classify::unreachable;
use crate::store::{Located, OpError, Store, StoreOps, VAULT_KIND};
use access::LedgerAccess;

/// How often a write retries after another device changed the same record.
const WRITE_ATTEMPTS: usize = 2;

pub(crate) struct Ledger {
    access: LedgerAccess,
    ops: Arc<dyn StoreOps>,
    groups: Groups,
}

/// A failure of a store call, as the vault reports it.
pub(super) fn vault_error(error: OpError) -> VaultError {
    match error {
        OpError::Conflict => VaultError::Conflict,
        OpError::Hub(HubError::NotSignedIn | HubError::Stale) => VaultError::Stale,
        OpError::Hub(HubError::PermissionDenied) => VaultError::ReadOnly,
        OpError::Damaged(_) => VaultError::Authentication,
        OpError::Hub(error) if unreachable(&error) => VaultError::Offline,
        OpError::Hub(error) => VaultError::Io(std::io::Error::other(error)),
    }
}

impl Ledger {
    #[cfg(test)]
    pub(crate) fn new(slot: Arc<Slot>, ops: Arc<dyn StoreOps>) -> Self {
        let groups = Groups::new(slot.cache_work().clone());
        Self {
            access: LedgerAccess::live(slot),
            ops,
            groups,
        }
    }

    pub(crate) fn captured(
        slot: Arc<Slot>,
        ops: Arc<dyn StoreOps>,
        capture: Arc<CapturedAccount>,
    ) -> Self {
        let groups = Groups::new(slot.cache_work().clone());
        Self {
            access: LedgerAccess::captured(slot, capture),
            ops,
            groups,
        }
    }

    fn store(&self, keys: &Arc<OpenKeys>) -> Store {
        Store::new(Arc::clone(&self.ops), Arc::clone(keys))
    }

    /// Whether the personal vault holds `object_id` live; a tombstone is held by no one.
    fn in_personal(&self, object_id: &str) -> Result<bool, VaultError> {
        Ok(self
            .cached()?
            .is_some_and(|vault| vault.objects.get(object_id).is_some_and(|o| !o.deleted)))
    }

    /// The group that holds `object_id`, unless the personal vault holds it.
    async fn group_owner(
        &self,
        store: &Store,
        object_id: &str,
    ) -> Result<Option<Owner>, VaultError> {
        if self.in_personal(object_id)? {
            return Ok(None);
        }
        self.groups.sync(store).await?;
        self.groups.owner_of(object_id)
    }

    /// Writes (`plaintext`) or deletes (`None`) a record of a group, with that group's key.
    /// A guest's write is refused before any call.
    pub(super) async fn write_group(
        &self,
        store: &Store,
        object_id: &str,
        plaintext: Option<&[u8]>,
        mut owner: Owner,
    ) -> Result<(), VaultError> {
        for attempt in 1..=WRITE_ATTEMPTS {
            if !owner.group.writable {
                return Err(VaultError::ReadOnly);
            }
            match (&owner.held, plaintext) {
                (Held::Unreadable, Some(_)) => return Err(VaultError::Format),
                (Held::Unreadable | Held::Deleted, None) => return Err(VaultError::NotFound),
                _ => {}
            }
            let at = &owner.group.at;
            let result = match plaintext {
                Some(plain) => store.put(at, KIND, object_id, plain, owner.version).await,
                None => store.remove(at, object_id, owner.version).await,
            };
            match result {
                Ok(_) => return self.groups.sync(store).await,
                Err(OpError::Conflict) if attempt < WRITE_ATTEMPTS => {
                    self.groups.sync(store).await?;
                    owner = self
                        .groups
                        .owner_in(owner.group.at.id(), object_id)?
                        .ok_or(VaultError::NotFound)?;
                }
                Err(error) => return Err(vault_error(error)),
            }
        }
        Err(VaultError::Conflict)
    }

    fn keys(&self) -> Result<Arc<OpenKeys>, VaultError> {
        self.access.keys()
    }

    /// Brings the cache up to date when the hub is reachable. Only a reachable hub's failure
    /// is an error.
    async fn sync(&self, keys: &Arc<OpenKeys>) -> Result<(), VaultError> {
        let turn = self.access.slot().refresh_turn(keys).await;
        self.keys()?;
        if !turn.needed() {
            return Ok(());
        }
        let store = Store::new(Arc::clone(&self.ops), Arc::clone(keys));
        let result = match store.find(VAULT_KIND).await {
            Ok(Some(at)) => self.pull(&store, &at).await,
            Ok(None) => Ok(()),
            Err(OpError::Hub(error)) if unreachable(&error) => Ok(()),
            Err(error) => Err(vault_error(error)),
        };
        if result.is_ok() {
            turn.complete();
        }
        result
    }

    /// Reads the changes after the cached position and writes the cache through.
    async fn pull(&self, store: &Store, at: &Located) -> Result<(), VaultError> {
        let copy = self.access.read(|cache| {
            cache
                .and_then(|cache| cache.vault.clone())
                .filter(|vault| vault.collection_id == at.id())
        })?;
        let mut vault = copy.unwrap_or_else(|| {
            Arc::new(VaultCopy {
                collection_id: at.id().to_owned(),
                key_envelope: at.envelope().to_vec(),
                cursor: 0,
                objects: BTreeMap::new(),
            })
        });
        let mut changed = false;
        let mut budget: Option<SnapshotBudget> = None;
        loop {
            let batch = store.changes(at, vault.cursor).await.map_err(vault_error)?;
            if !changed
                && batch.changes.is_empty()
                && !batch.more
                && batch.head <= vault.cursor
                && vault.key_envelope == at.envelope()
            {
                return Ok(());
            }
            changed = true;
            let envelope = at.envelope().to_vec();
            let merged = self
                .access
                .slot()
                .cache_work()
                .run(move || Ok(merge_vault(vault, budget, envelope, batch)))
                .await
                .map_err(|error| VaultError::Io(std::io::Error::other(error)))?
                .map_err(vault_error)?;
            vault = merged.0;
            budget = Some(merged.1);
            if !merged.2 {
                break;
            }
        }
        self.access.publish_vault(vault).await?;
        Ok(())
    }

    /// Opens one cached object: `Ok(None)` when it is a tombstone or another kind, and an
    /// error when it does not authenticate.
    fn open(
        keys: &OpenKeys,
        collection: &str,
        envelope: &[u8],
        object_id: &str,
        object: &ObjectCopy,
    ) -> Result<Option<Decoded>, VaultError> {
        if object.deleted || object.kind != KIND {
            return Ok(None);
        }
        let key = keys
            .own_collection(collection, envelope)
            .map_err(|_| VaultError::Authentication)?;
        codec::open_sealed(&key, collection, object_id, object)
    }

    fn cached(&self) -> Result<Option<Arc<VaultCopy>>, VaultError> {
        self.access
            .read(|cache| cache.and_then(|cache| cache.vault.clone()))
    }

    pub(super) async fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        let _operation = self.access.operation().await?;
        let keys = self.keys()?;
        self.sync(&keys).await?;
        let Some(vault) = self.cached()? else {
            return Ok(Vec::new());
        };
        let mut listed = Vec::new();
        for (object_id, object) in &vault.objects {
            if let Some(Decoded::Known(record)) = Self::open(
                &keys,
                &vault.collection_id,
                &vault.key_envelope,
                object_id,
                object,
            )? && let Ok(id) = object_id.parse::<CredentialId>()
            {
                listed.push(CredentialInfo {
                    id,
                    label: record.label,
                    binding: record.binding,
                    group: None,
                });
            }
        }
        self.groups.sync(&self.store(&keys)).await?;
        // One row per id: the one `get`, `put` and `delete` answer for.
        let mut seen: HashSet<CredentialId> = listed.iter().map(|info| info.id).collect();
        listed.extend(
            self.groups
                .list()?
                .into_iter()
                .filter(|info| seen.insert(info.id)),
        );
        Ok(listed)
    }

    pub(super) async fn get(
        &self,
        id: CredentialId,
        binding: CredentialBinding,
    ) -> Result<Secret, VaultError> {
        let _operation = self.access.operation().await?;
        let keys = self.keys()?;
        self.sync(&keys).await?;
        let object_id = id.to_string();
        let personal = self.access.read(|cache| {
            let vault = cache?.vault.as_ref()?;
            let object = vault
                .objects
                .get(&object_id)
                .filter(|object| !object.deleted)?;
            Some((
                vault.collection_id.clone(),
                vault.key_envelope.clone(),
                object.clone(),
            ))
        })?;
        let Some((collection, envelope, object)) = personal else {
            self.groups.sync(&self.store(&keys)).await?;
            return self.groups.secret(&object_id, &binding);
        };
        match Self::open(&keys, &collection, &envelope, &object_id, &object)? {
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
        let _operation = self.access.operation().await?;
        let keys = self.keys()?;
        let store = self.store(&keys);
        let given = id.is_some();
        let id = match id {
            Some(id) => id,
            None => {
                CredentialId::from_bytes(random_bytes::<16>().map_err(|_| VaultError::Randomness)?)
            }
        };
        let object_id = id.to_string();
        let plaintext = codec::encode(&label, &binding, &secret).map_err(|_| VaultError::Format)?;
        if given && let Some(owner) = self.group_owner(&store, &object_id).await? {
            self.write_group(&store, &object_id, Some(&plaintext), owner)
                .await?;
            return Ok(id);
        }
        self.write_personal(&keys, &store, &object_id, &plaintext)
            .await?;
        Ok(id)
    }

    /// Writes a record of the personal vault, whatever a group holds under the same id.
    pub(super) async fn write_personal(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        object_id: &str,
        plaintext: &[u8],
    ) -> Result<(), VaultError> {
        for attempt in 1..=WRITE_ATTEMPTS {
            let at = store.ensure(VAULT_KIND).await.map_err(vault_error)?;
            self.pull(store, &at).await?;
            let expected = self.expected(keys, object_id, true)?;
            match store.put(&at, KIND, object_id, plaintext, expected).await {
                Ok(_) => return self.pull(store, &at).await,
                Err(OpError::Conflict) if attempt < WRITE_ATTEMPTS => {}
                Err(error) => return Err(vault_error(error)),
            }
        }
        Err(VaultError::Conflict)
    }

    pub(super) async fn delete(&self, id: CredentialId) -> Result<(), VaultError> {
        let _operation = self.access.operation().await?;
        let keys = self.keys()?;
        let store = self.store(&keys);
        let object_id = id.to_string();
        if let Some(owner) = self.group_owner(&store, &object_id).await? {
            return self.write_group(&store, &object_id, None, owner).await;
        }
        self.remove_personal(&keys, &store, &object_id).await
    }

    /// Deletes a record of the personal vault, whatever a group holds under the same id.
    pub(super) async fn remove_personal(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        object_id: &str,
    ) -> Result<(), VaultError> {
        for attempt in 1..=WRITE_ATTEMPTS {
            let at = store
                .find(VAULT_KIND)
                .await
                .map_err(vault_error)?
                .ok_or(VaultError::NotFound)?;
            self.pull(store, &at).await?;
            let expected = self.expected(keys, object_id, false)?;
            match store.remove(&at, object_id, expected).await {
                Ok(_) => return self.pull(store, &at).await,
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
        let Some(vault) = self.cached()? else {
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
        match Self::open(
            keys,
            &vault.collection_id,
            &vault.key_envelope,
            object_id,
            object,
        )? {
            Some(Decoded::Known(_)) => Ok(object.version),
            _ if creating => Err(VaultError::Format),
            _ => Err(VaultError::NotFound),
        }
    }
}

mod carry;
pub(crate) use carry::Space;

#[cfg(test)]
mod tests;
