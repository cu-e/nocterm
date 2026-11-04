//! Moving credentials from one space to another: the personal vault, or a group's.
//!
//! A credential moves by being written into the target space, sealed with that space's key,
//! and then deleted from the space it leaves. A space the account may not write refuses the
//! move before anything is copied, so a secret is never left in two places by a refusal,
//! and never reaches a vault that was not asked for.
//!
//! A credential another profile that stays behind still uses is copied instead: it is written
//! into the target under a new id and left in the space it came from. Each space then holds
//! the secret under an id of its own, so that an edit or a deletion reaches the copy it was
//! made on and no other; the caller learns the new id and points the profile that follows at it.

use std::sync::Arc;

use nocterm_keys::random_bytes;
use nocterm_session::CredentialId;
use nocterm_vault::VaultError;

use super::Ledger;
use crate::account::OpenKeys;
use crate::store::Store;
use crate::vault::codec::{self, Decoded, Record};
use crate::vault::groups::Held;

/// What becomes of the credential in the space it comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leave {
    /// It is deleted there: the credential moves.
    Remove,
    /// It stays there as well: the credential is copied.
    Keep,
}

/// A vault of the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    Personal,
    /// The vault of the organization with this id.
    Group(String),
}

impl Ledger {
    /// Moves the credentials `ids` that `from` holds into `to`, and answers the ones that
    /// moved; one that `from` does not hold stays where it is. When a move fails, the ones
    /// that already moved go back, as far as that is possible.
    pub(crate) async fn carry(
        &self,
        from: &Space,
        to: &Space,
        ids: &[CredentialId],
    ) -> Result<Vec<CredentialId>, VaultError> {
        let moved = self.relocate(from, to, ids, Leave::Remove).await?;
        Ok(moved.into_iter().map(|(id, _)| id).collect())
    }

    /// Copies the credentials `ids` that `from` holds into `to` under new ids, which leaves
    /// them in `from` too, and answers each copy as `(id in from, id in to)`. When a copy
    /// fails, the ones already copied are removed from `to` again.
    pub(crate) async fn copy(
        &self,
        from: &Space,
        to: &Space,
        ids: &[CredentialId],
    ) -> Result<Vec<(CredentialId, CredentialId)>, VaultError> {
        self.relocate(from, to, ids, Leave::Keep).await
    }

    async fn relocate(
        &self,
        from: &Space,
        to: &Space,
        ids: &[CredentialId],
        leave: Leave,
    ) -> Result<Vec<(CredentialId, CredentialId)>, VaultError> {
        let keys = self.keys()?;
        let store = self.store(&keys);
        self.sync(&keys).await?;
        self.groups.sync(&store).await?;
        let mut moved: Vec<(CredentialId, CredentialId)> = Vec::new();
        for id in ids {
            match self.carry_one(&keys, &store, from, to, *id, leave).await {
                Ok(Some(arrived)) => moved.push((*id, arrived)),
                Ok(None) => {}
                Err(error) => {
                    for (origin, arrived) in &moved {
                        let _ = match leave {
                            Leave::Remove => self
                                .carry_one(&keys, &store, to, from, *origin, leave)
                                .await
                                .map(drop),
                            Leave::Keep => {
                                self.remove_in(&keys, &store, to, &arrived.to_string())
                                    .await
                            }
                        };
                    }
                    return Err(error);
                }
            }
        }
        Ok(moved)
    }

    /// The id the credential `id` goes by in `to` when it was in `from` and is now in `to`,
    /// and in `from` no more unless `leave` keeps it there: a moved credential keeps its id,
    /// a copied one gets a new id.
    async fn carry_one(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        from: &Space,
        to: &Space,
        id: CredentialId,
        leave: Leave,
    ) -> Result<Option<CredentialId>, VaultError> {
        let object_id = id.to_string();
        let Some(record) = self.read_in(keys, from, &object_id)? else {
            return Ok(None);
        };
        let arrived = match leave {
            Leave::Remove => id,
            Leave::Keep => {
                CredentialId::from_bytes(random_bytes::<16>().map_err(|_| VaultError::Randomness)?)
            }
        };
        let target_id = arrived.to_string();
        self.require_writable(from, &object_id)?;
        self.require_writable(to, &target_id)?;
        let plaintext = codec::encode(&record.label, &record.binding, &record.secret)
            .map_err(|_| VaultError::Format)?;
        self.write_in(keys, store, to, &target_id, &plaintext)
            .await?;
        if leave == Leave::Keep {
            return Ok(Some(arrived));
        }
        if let Err(error) = self.remove_in(keys, store, from, &object_id).await {
            let _ = self.remove_in(keys, store, to, &target_id).await;
            return Err(error);
        }
        Ok(Some(arrived))
    }

    /// The credential as `space` holds it live, if it does.
    fn read_in(
        &self,
        keys: &OpenKeys,
        space: &Space,
        object_id: &str,
    ) -> Result<Option<Record>, VaultError> {
        match space {
            Space::Personal => {
                let Some(vault) = self.cached()? else {
                    return Ok(None);
                };
                let Some(object) = vault.objects.get(object_id) else {
                    return Ok(None);
                };
                Ok(
                    match Self::open(
                        keys,
                        &vault.collection_id,
                        &vault.key_envelope,
                        object_id,
                        object,
                    )? {
                        Some(Decoded::Known(record)) => Some(record),
                        Some(Decoded::Unrecognized) | None => None,
                    },
                )
            }
            Space::Group(org_id) => {
                let Some(owner) = self.groups.target(org_id, object_id)? else {
                    return Ok(None);
                };
                self.groups.record(owner.group.at.id(), object_id)
            }
        }
    }

    /// A group the account only reads, or holds no key of, cannot be written.
    fn require_writable(&self, space: &Space, object_id: &str) -> Result<(), VaultError> {
        match space {
            Space::Personal => Ok(()),
            Space::Group(org_id) => match self.groups.target(org_id, object_id)? {
                Some(owner) if owner.group.writable => Ok(()),
                _ => Err(VaultError::ReadOnly),
            },
        }
    }

    async fn write_in(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        space: &Space,
        object_id: &str,
        plaintext: &[u8],
    ) -> Result<(), VaultError> {
        match space {
            Space::Personal => self.write_personal(keys, store, object_id, plaintext).await,
            Space::Group(org_id) => {
                let owner = self
                    .groups
                    .target(org_id, object_id)?
                    .ok_or(VaultError::ReadOnly)?;
                self.write_group(store, object_id, Some(plaintext), owner)
                    .await
            }
        }
    }

    async fn remove_in(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        space: &Space,
        object_id: &str,
    ) -> Result<(), VaultError> {
        match space {
            Space::Personal => self.remove_personal(keys, store, object_id).await,
            Space::Group(org_id) => {
                let owner = self
                    .groups
                    .target(org_id, object_id)?
                    .ok_or(VaultError::ReadOnly)?;
                if !matches!(owner.held, Held::Known) {
                    return Err(VaultError::NotFound);
                }
                self.write_group(store, object_id, None, owner).await
            }
        }
    }
}
