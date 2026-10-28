//! Unlocking the vault: the typed password opens the keyset, from the hub's record when the
//! hub answers and from the cached record when it does not.

use std::sync::Arc;

use nocterm_hub_client::{Keyset, KeysetError, UnlockMethod as KeysetUnlock};
use nocterm_hub_proto::v1;
use nocterm_keys::{LoginError, LoginSecrets, UnlockKey};
use nocterm_session::Secret;
use nocterm_vault::VaultError;

use super::ledger::vault_error;
use crate::account::{OpenKeys, Slot};
use crate::cache::{AccountCache, KeysetCopy, UnlockMethod};
use crate::classify::unreachable;
use crate::store::{OpError, StoreOps};

pub(super) struct Unlocker {
    slot: Arc<Slot>,
    ops: Arc<dyn StoreOps>,
}

impl Unlocker {
    pub(super) fn new(slot: Arc<Slot>, ops: Arc<dyn StoreOps>) -> Self {
        Self { slot, ops }
    }

    pub(super) async fn unlock(&self, password: Secret) -> Result<(), VaultError> {
        if self.slot.open().is_some() {
            return Ok(());
        }
        let cached = self
            .slot
            .read_cache(|cache| cache.cloned())
            .ok_or(VaultError::NotFound)?;
        let record = self.record(&cached).await?;
        let unlock = derive(&cached, &record, password).await?;
        let keyset =
            Keyset::open(&record, &cached.account_id, &unlock).map_err(|error| match error {
                KeysetError::WrongUnlockKey | KeysetError::Damaged(_) => VaultError::Authentication,
                _ => VaultError::Format,
            })?;
        let fresh = KeysetCopy::of(&record);
        if fresh != cached.keyset
            && let Err(error) = self.slot.update_cache(|cache| cache.keyset = fresh)
        {
            tracing::warn!(%error, "the account cache cannot be written");
        }
        self.slot
            .unlocked(OpenKeys::pinning(keyset, cached.pinned()));
        Ok(())
    }

    /// The keyset record to open: the hub's when it answers, else the cached copy.
    async fn record(&self, cached: &AccountCache) -> Result<v1::AccountKeyset, VaultError> {
        match self.ops.keyset().await {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Ok(cached.keyset.record()),
            Err(OpError::Hub(error)) if unreachable(&error) => Ok(cached.keyset.record()),
            Err(error) => Err(vault_error(error)),
        }
    }
}

/// The unlock key of the typed password, derived the way the keyset records ([`KeysetUnlock`]),
/// not the way the person happened to sign in. Key derivation is slow and memory hard, so it
/// runs on a thread of its own.
async fn derive(
    cached: &AccountCache,
    record: &v1::AccountKeyset,
    password: Secret,
) -> Result<UnlockKey, VaultError> {
    let method = KeysetUnlock::of(record).map_err(|_| VaultError::Format)?;
    let hub_uid = cached.hub_uid.clone();
    // Only a login password needs the user name, which the cache kept from the sign-in.
    let username = match &cached.method {
        UnlockMethod::Login { username } => Some(username.clone()),
        UnlockMethod::Passphrase => None,
    };
    let derived = tokio::task::spawn_blocking(move || match (method, username) {
        (KeysetUnlock::LoginPassword, Some(username)) => {
            LoginSecrets::derive(password.expose(), &hub_uid, &username)
                .map(LoginSecrets::into_unlock)
        }
        (KeysetUnlock::LoginPassword, None) => Err(LoginError::EmptyUsername),
        (KeysetUnlock::VaultPassword(kdf), _) => {
            UnlockKey::from_vault_password(password.expose(), &kdf)
        }
    })
    .await
    .map_err(|_| VaultError::Cancelled)?;
    derived.map_err(|error| match error {
        LoginError::PasswordTooShort { .. } => VaultError::Password,
        LoginError::EmptyUsername => VaultError::Format,
        LoginError::Key(_) => VaultError::Randomness,
    })
}

#[cfg(test)]
mod tests;
