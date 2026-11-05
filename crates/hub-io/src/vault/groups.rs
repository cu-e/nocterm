//! The credentials of organization groups: sealed snapshots read through from the hub.
//!
//! A group's vault is a collection of the organization that an administrator granted the
//! account the key of. Its records are kept in memory only, sealed as the hub holds them, and
//! opened on use; the encrypted file cache stays the account's own. A record is routed by the
//! collection it was read from, so a group's credential is never written to the personal vault
//! and never listed as personal. A guest only reads: its writes are
//! [`VaultError::ReadOnly`] without a call.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_hubs_model::HubError;
use nocterm_session::Secret;
use nocterm_vault::{CredentialBinding, CredentialGroup, CredentialInfo, VaultError};

use super::codec::{self, Decoded, Record};
use super::ledger::vault_error;
use super::snapshot_budget::{MAX_GROUPS, SnapshotBudget};
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

pub(super) struct Groups {
    held: Mutex<HashMap<String, Arc<Snapshot>>>,
    work: Arc<crate::cache::CacheWork>,
}

impl Groups {
    pub(super) fn new(work: Arc<crate::cache::CacheWork>) -> Self {
        Self {
            held: Mutex::default(),
            work,
        }
    }
    fn held(&self) -> MutexGuard<'_, HashMap<String, Arc<Snapshot>>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn snapshots(&self) -> Vec<Arc<Snapshot>> {
        self.held().values().cloned().collect()
    }

    /// Brings the snapshots up to date with the groups the account holds keys of. An
    /// unreachable hub leaves them as they are; a group that was taken away is dropped.
    pub(super) async fn sync(&self, store: &Store) -> Result<(), VaultError> {
        let layout = match store.layout_bounded(VAULT_KIND, MAX_GROUPS).await {
            Ok(layout) => layout,
            Err(OpError::Hub(error)) if unreachable(&error) => return Ok(()),
            Err(error) => return Err(vault_error(error)),
        };
        let mut next = HashMap::new();
        if layout.groups.len() > MAX_GROUPS {
            return Err(VaultError::Format);
        }
        let mut budget = SnapshotBudget::default();
        for group in layout.groups {
            let id = group.at.id().to_owned();
            let original = self.held().get(&id).cloned();
            let prepared = self
                .work
                .run(move || {
                    let mut snapshot = original.as_deref().cloned().unwrap_or_else(|| Snapshot {
                        group: group.clone(),
                        cursor: 0,
                        objects: BTreeMap::new(),
                    });
                    snapshot.group = group;
                    let admitted = budget.add(&snapshot.objects);
                    Ok((snapshot, budget, admitted))
                })
                .await
                .map_err(|error| VaultError::Io(std::io::Error::other(error)))?;
            let mut snapshot = prepared.0;
            budget = prepared.1;
            prepared.2.map_err(vault_error)?;
            match Self::pull(store, &mut snapshot, &mut budget).await {
                Ok(()) => {}
                Err(OpError::Hub(error)) if unreachable(&error) => {}
                Err(OpError::Hub(HubError::NotFound | HubError::PermissionDenied)) => continue,
                Err(error) => return Err(vault_error(error)),
            }
            next.insert(id, Arc::new(snapshot));
        }
        *self.held() = next;
        Ok(())
    }

    async fn pull(
        store: &Store,
        snapshot: &mut Snapshot,
        budget: &mut SnapshotBudget,
    ) -> Result<(), OpError> {
        loop {
            let batch = store.changes(&snapshot.group.at, snapshot.cursor).await?;
            if !budget.apply(&mut snapshot.objects, &mut snapshot.cursor, batch)? {
                return Ok(());
            }
        }
    }

    /// Every credential of every group, marked with its group and the member's access.
    pub(super) fn list(&self) -> Result<Vec<CredentialInfo>, VaultError> {
        let mut snapshots = self.snapshots();
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
        for snapshot in self.snapshots() {
            let Some(owner) = Self::owner_in_snapshot(&snapshot, object_id)? else {
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
        let snapshot = self.held().get(collection_id).cloned();
        match snapshot {
            Some(snapshot) => Self::owner_in_snapshot(&snapshot, object_id),
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
        let snapshots = self.snapshots();
        let Some(snapshot) = snapshots
            .iter()
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
        let snapshot = self.held().get(collection_id).cloned();
        let Some(snapshot) = snapshot else {
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

    /// The secret of a group's credential, when it answers for `binding`. A group that
    /// only keeps a tombstone of it, such as the one it was moved out of, does not answer.
    pub(super) fn secret(
        &self,
        object_id: &str,
        binding: &CredentialBinding,
    ) -> Result<Secret, VaultError> {
        for snapshot in self.snapshots() {
            let Some(object) = snapshot.objects.get(object_id) else {
                continue;
            };
            if object.deleted {
                continue;
            }
            return match snapshot.open(object_id, object)? {
                Some(Decoded::Known(record)) if record.binding == *binding => Ok(record.secret),
                Some(Decoded::Known(_)) => Err(VaultError::Binding),
                Some(Decoded::Unrecognized) | None => Err(VaultError::NotFound),
            };
        }
        Err(VaultError::NotFound)
    }
}
