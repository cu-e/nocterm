//! Unlocking the vault: the typed password opens the keyset, from the hub's record when the
//! hub answers and from the cached record when it does not.

use std::sync::Arc;

use nocterm_hub_client::{Keyset, KeysetError, UnlockMethod as KeysetUnlock};
use nocterm_hub_proto::v2;
use nocterm_keys::{LoginError, LoginSecrets, UnlockKey};
use nocterm_session::Secret;
use nocterm_vault::VaultError;

use super::ledger::vault_error;
use super::native_failure as failure;
use crate::account::{OpenKeys, SlotReceipt, operations::MembershipWork};
use crate::cache::{AccountCache, KeysetCopy, UnlockMethod};
use crate::classify::unreachable;
use crate::store::OpError;

pub(super) struct Unlocker {
    work: MembershipWork,
}

impl Unlocker {
    pub(super) fn new(work: MembershipWork) -> Self {
        Self { work }
    }

    pub(super) async fn unlock(&self, password: Secret) -> Result<SlotReceipt, VaultError> {
        self.work.check().map_err(failure)?;
        let slot = self.work.slot().map_err(failure)?;
        let member = self.work.membership();
        let subject = member.subject().map_err(failure)?;
        if slot.open_for(&subject).is_ok() {
            return Ok(self.work.receipt().clone());
        }
        let mut cached = member
            .with_current(|| slot.cache_for(&subject, self.work.receipt()))
            .map_err(failure)?;
        let record = self.record(&cached).await?;
        let unlock = derive(&cached, &record, password).await?;
        self.work.check().map_err(failure)?;
        let keyset =
            Keyset::open(&record, &cached.account_id, &unlock).map_err(|error| match error {
                KeysetError::WrongUnlockKey | KeysetError::Damaged(_) => VaultError::Authentication,
                _ => VaultError::Format,
            })?;
        cached.keyset = KeysetCopy::of(&record);
        let _turn = slot.cache_turn().await;
        self.work.check().map_err(failure)?;
        let original = slot.cache_snapshot();
        if let Some(latest) = original.as_deref() {
            cached.vault = latest.vault.clone();
            cached.recovery_unconfirmed = latest.recovery_unconfirmed;
        }
        let keys = Arc::new(OpenKeys::new(keyset));
        let draft = slot.stage_cache(original, cached).await.map_err(failure)?;
        let (receipt, committed) = member
            .with_current(|| slot.commit_install(&subject, self.work.receipt(), keys, draft))
            .map_err(failure)?
            .ok_or(VaultError::Stale)?;
        slot.finish_cache(committed).await;
        slot.notice().raise();
        Ok(receipt)
    }

    /// The keyset record to open: the hub's when it answers, else the cached copy.
    async fn record(&self, cached: &AccountCache) -> Result<v2::Keyset, VaultError> {
        match self.work.ops().keyset().await {
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
    record: &v2::Keyset,
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
