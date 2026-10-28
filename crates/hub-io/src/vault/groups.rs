//! The credentials of organization groups: sealed snapshots read through from the hub.
//!
//! A group's vault is a collection of the organization that an administrator granted the
//! account the key of. Its records are kept in memory only, sealed as the hub holds them, and
//! opened on use; the encrypted file cache stays the account's own. A record is routed by the
//! collection it was read from, so a group's credential is never written to the personal vault
//! and never listed as personal. A guest only reads: its writes are
//! [`VaultError::ReadOnly`] without a call.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use nocterm_hubs_model::HubError;
use nocterm_session::Secret;
use nocterm_vault::{CredentialBinding, CredentialGroup, CredentialInfo, VaultError};

use super::codec::{self, Decoded, Record};
use super::ledger::vault_error;
use crate::cache::ObjectCopy;
use crate::classify::unreachable;
use crate::store::{Group, OpError, Store, VAULT_KIND};

/// One group's vault as the last read left it.
#[derive(Clone)]
struct Snapshot {
    group: Group,
    cursor: u64,
    objects: BTreeMap<String, ObjectCopy>,
}

impl Snapshot {
    fn open(&self, object_id: &str, object: &ObjectCopy) -> Result<Option<Decoded>, VaultError> {
        codec::open_sealed(self.group.at.key(), self.group.at.id(), object_id, object)
    }
}

/// What a group holds under a credential id.
pub(super) enum Held {
    /// A credential this client reads.
    Known,
    /// The record was deleted; its version still guards a new write.
    Deleted,
    /// A record of a newer format: never overwritten or deleted.
    Unreadable,
}

/// The group that holds a credential id, and what a write to it must expect.
pub(super) struct Owner {
    pub(super) group: Group,
    pub(super) version: u64,
    pub(super) held: Held,
}

#[derive(Default)]
pub(super) struct Groups {
    held: Mutex<HashMap<String, Snapshot>>,
}

impl Groups {
    fn held(&self) -> MutexGuard<'_, HashMap<String, Snapshot>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Brings the snapshots up to date with the groups the account holds keys of. An
    /// unreachable hub leaves them as they are; a group that was taken away is dropped.
    pub(super) async fn sync(&self, store: &Store) -> Result<(), VaultError> {
        let layout = match store.layout(VAULT_KIND).await {
            Ok(layout) => layout,
            Err(OpError::Hub(error)) if unreachable(&error) => return Ok(()),
            Err(error) => return Err(vault_error(error)),
        };
        let mut next = HashMap::new();
        for group in layout.groups {
            let id = group.at.id().to_owned();
            let mut snapshot = self.held().get(&id).cloned().unwrap_or_else(|| Snapshot {
                group: group.clone(),
                cursor: 0,
                objects: BTreeMap::new(),
            });
            snapshot.group = group;
            match Self::pull(store, &mut snapshot).await {
                Ok(()) => {}
                Err(OpError::Hub(error)) if unreachable(&error) => {}
                Err(OpError::Hub(HubError::NotFound | HubError::PermissionDenied)) => continue,
                Err(error) => return Err(vault_error(error)),
            }
            next.insert(id, snapshot);
        }
        *self.held() = next;
        Ok(())
    }

    async fn pull(store: &Store, snapshot: &mut Snapshot) -> Result<(), OpError> {
        loop {
            let batch = store.changes(&snapshot.group.at, snapshot.cursor).await?;
            for object in batch.changes {
                snapshot.cursor = snapshot.cursor.max(object.stream_version);
                snapshot.objects.insert(
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
                snapshot.cursor = snapshot.cursor.max(batch.head);
                return Ok(());
            }
        }
    }

    /// Every credential of every group, marked with its group and the member's access.
    pub(super) fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        let mut snapshots: Vec<Snapshot> = self.held().values().cloned().collect();
        snapshots.sort_by(|a, b| {
            (&a.group.name, &a.group.org_id).cmp(&(&b.group.name, &b.group.org_id))
        });
        let mut listed = Vec::new();
        for snapshot in &snapshots {
            for (object_id, object) in &snapshot.objects {
                if let Some(Decoded::Known(record)) = snapshot.open(object_id, object)?
                    && let Ok(id) = object_id.parse()
                {
                    listed.push(CredentialInfo {
                        id,
                        label: record.label,
                        binding: record.binding,
                        group: Some(CredentialGroup {
                            name: snapshot.group.name.clone(),
                            writable: snapshot.group.writable,
                        }),
                    });
                }
            }
        }
        Ok(listed)
    }

    /// The group that holds `object_id`, if any. A group that holds it live answers before
    /// one that only keeps a tombstone of it, such as the group it was moved out of.
    pub(super) fn owner_of(&self, object_id: &str) -> Result<Option<Owner>, VaultError> {
        let mut deleted = None;
        for snapshot in self.held().values() {
            let Some(owner) = Self::owner_in_snapshot(snapshot, object_id)? else {
                continue;
            };
            if !matches!(owner.held, Held::Deleted) {
                return Ok(Some(owner));
            }
            deleted.get_or_insert(owner);
        }
        Ok(deleted)
    }

    /// What the group of collection `collection_id` holds under `object_id`, if anything.
    pub(super) fn owner_in(
        &self,
        collection_id: &str,
        object_id: &str,
    ) -> Result<Option<Owner>, VaultError> {
        match self.held().get(collection_id) {
            Some(snapshot) => Self::owner_in_snapshot(snapshot, object_id),
            None => Ok(None),
        }
    }

    /// A write target in the group of organization `org_id`: what it holds under
    /// `object_id`, or a fresh record when it holds nothing. `None` when the account holds
    /// no key of that group.
    pub(super) fn target(
        &self,
        org_id: &str,
        object_id: &str,
    ) -> Result<Option<Owner>, VaultError> {
        let held = self.held();
        let Some(snapshot) = held
            .values()
            .find(|snapshot| snapshot.group.org_id == org_id)
        else {
            return Ok(None);
        };
        Ok(Some(match Self::owner_in_snapshot(snapshot, object_id)? {
            Some(owner) => owner,
            None => Owner {
                group: snapshot.group.clone(),
                version: 0,
                held: Held::Deleted,
            },
        }))
    }

    fn owner_in_snapshot(
        snapshot: &Snapshot,
        object_id: &str,
    ) -> Result<Option<Owner>, VaultError> {
        let Some(object) = snapshot.objects.get(object_id) else {
            return Ok(None);
        };
        let held = if object.deleted {
            Held::Deleted
        } else {
            match snapshot.open(object_id, object)? {
                Some(Decoded::Known(_)) => Held::Known,
                _ => Held::Unreadable,
            }
        };
        Ok(Some(Owner {
            group: snapshot.group.clone(),
            version: object.version,
            held,
        }))
    }

    /// The credential `object_id` as the group of `collection_id` holds it, when it is
    /// held live and readable.
    pub(super) fn record(
        &self,
        collection_id: &str,
        object_id: &str,
    ) -> Result<Option<Record>, VaultError> {
        let held = self.held();
        let Some(snapshot) = held.get(collection_id) else {
            return Ok(None);
        };
        let Some(object) = snapshot.objects.get(object_id) else {
            return Ok(None);
        };
        Ok(match snapshot.open(object_id, object)? {
            Some(Decoded::Known(record)) => Some(record),
            Some(Decoded::Unrecognized) | None => None,
        })
    }

    /// The secret of a group's credential, when it answers for `binding`.
    pub(super) fn secret(
        &self,
        object_id: &str,
        binding: &CredentialBinding,
    ) -> Result<Secret, VaultError> {
        for snapshot in self.held().values() {
            let Some(object) = snapshot.objects.get(object_id) else {
                continue;
            };
            return match snapshot.open(object_id, object)? {
                Some(Decoded::Known(record)) if record.binding == *binding => Ok(record.secret),
                Some(Decoded::Known(_)) => Err(VaultError::Binding),
                Some(Decoded::Unrecognized) | None => Err(VaultError::NotFound),
            };
        }
        Err(VaultError::NotFound)
    }
}
