//! Moving credentials from one space to another: the personal vault, or a group's.
//!
//! A credential moves by being written into the target space, sealed with that space's key,
//! and then deleted from the space it leaves. A space the account may not write refuses the
//! move before anything is copied, so a secret is never left in two places by a refusal,
//! and never reaches a vault that was not asked for.

use std::sync::Arc;

use nocterm_session::CredentialId;
use nocterm_vault::VaultError;

use super::Ledger;
use crate::account::OpenKeys;
use crate::store::Store;
use crate::vault::codec::{self, Decoded, Record};
use crate::vault::groups::Held;

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
        let keys = self.keys()?;
        let store = self.store(&keys);
        self.sync(&keys).await?;
        self.groups.sync(&store).await?;
        let mut moved = Vec::new();
        for id in ids {
            match self
                .carry_one(&keys, &store, from, to, &id.to_string())
                .await
            {
                Ok(true) => moved.push(*id),
                Ok(false) => {}
                Err(error) => {
                    for done in &moved {
                        let _ = self
                            .carry_one(&keys, &store, to, from, &done.to_string())
                            .await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(moved)
    }

    /// Whether the credential `object_id` was in `from` and is now in `to` alone.
    async fn carry_one(
        &self,
        keys: &Arc<OpenKeys>,
        store: &Store,
        from: &Space,
        to: &Space,
        object_id: &str,
    ) -> Result<bool, VaultError> {
        let Some(record) = self.read_in(keys, from, object_id)? else {
            return Ok(false);
        };
        self.require_writable(from, object_id)?;
        self.require_writable(to, object_id)?;
        let plaintext = codec::encode(&record.label, &record.binding, &record.secret)
            .map_err(|_| VaultError::Format)?;
        self.write_in(keys, store, to, object_id, &plaintext)
            .await?;
        if let Err(error) = self.remove_in(keys, store, from, object_id).await {
            let _ = self.remove_in(keys, store, to, object_id).await;
            return Err(error);
        }
        Ok(true)
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
                let Some(vault) = self.cached() else {
                    return Ok(None);
                };
                let Some(object) = vault.objects.get(object_id) else {
                    return Ok(None);
                };
                Ok(match Self::open(keys, &vault, object_id, object)? {
                    Some(Decoded::Known(record)) => Some(record),
                    Some(Decoded::Unrecognized) | None => None,
                })
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
