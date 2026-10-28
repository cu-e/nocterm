//! Opening, creating and recovering the account keys around a sign-in.
//!
//! A first sign-in creates the keyset (one conditional write: a second device that races it
//! opens the keyset that won, it never overwrites it). A later sign-in opens it with the key
//! derived from the password. A password that does not open it (it was reset) leaves the keys
//! waiting for the recovery key. A keyset that does not authenticate although the key is right
//! was altered by the hub: that is an error and nothing is written.

use std::sync::Arc;

use futures::future::BoxFuture;
use nocterm_core::SharedCause;
use nocterm_hub_client::{Keyset, KeysetError};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{HubError, HubId, KeyStatus};
use nocterm_keys::{LoginError, LoginSecrets, RecoveryKey, UnlockKdf, UnlockKey};
use nocterm_session::Secret;

use super::keyset_hub::KeysetHub;
use super::{Accounts, OpenKeys, Recovery, Slot};
use crate::cache::{AccountCache, KeysetCopy, UnlockMethod};
use crate::classify::{credentials, remote};
use crate::directory::who_am_i;
use crate::pool::Pool;

/// The key work of a sign-in, for every hub.
pub(crate) struct KeyFlow {
    pool: Arc<Pool>,
    accounts: Arc<Accounts>,
}

/// How often the replacement of the recovery envelope is tried against a keyset that other
/// devices keep writing.
const RESEAL_ATTEMPTS: u32 = 4;

/// What a keyset write decided.
enum Creation {
    Created(nocterm_hub_client::NewKeyset),
    /// Another device created the keyset first.
    Raced,
}

/// A keyset that someone else already created is opened, not replaced.
fn creation(
    result: Result<nocterm_hub_client::NewKeyset, KeysetError>,
) -> Result<Creation, KeysetError> {
    match result {
        Ok(created) => Ok(Creation::Created(created)),
        Err(KeysetError::AlreadyExists) => Ok(Creation::Raced),
        Err(error) => Err(error),
    }
}

impl KeyFlow {
    pub(crate) fn new(pool: Arc<Pool>, accounts: Arc<Accounts>) -> Self {
        Self { pool, accounts }
    }

    /// The keys after a password sign-in: the login password also derives the unlock key.
    pub(crate) async fn after_password(
        &self,
        hub: &HubId,
        client: &dyn KeysetHub,
        account_id: &str,
        username: &str,
        secrets: LoginSecrets,
    ) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        let kdf = secrets.kdf().clone();
        let unlock = secrets.into_unlock();
        let method = UnlockMethod::Login {
            username: username.trim().to_owned(),
        };
        self.settle(&slot, client, account_id, method, unlock, kdf)
            .await
    }

    /// The keys after a browser sign-in: nothing opens them until a passphrase is typed.
    pub(crate) fn after_browser(&self, hub: &HubId) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        Ok(match slot.open() {
            Some(keys) => opened(&slot, keys.keyset().account_id()),
            None => KeyStatus::PassphraseNeeded,
        })
    }

    /// Whether the recovery key made on this device was never confirmed as saved.
    pub(crate) fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        let Ok(slot) = self.accounts.slot(hub) else {
            return false;
        };
        slot.open()
            .is_some_and(|keys| slot.recovery_unconfirmed(keys.keyset().account_id()))
    }

    /// The count of changes to any hub's account keys (opened, dropped, replaced) so far.
    pub(crate) fn keys_revision(&self) -> u64 {
        self.accounts.notice().count()
    }

    /// Resolves with the count once it differs from `seen`.
    pub(crate) fn keys_changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let mut changes = self.accounts.notice().subscribe();
        Box::pin(async move {
            loop {
                let now = *changes.borrow_and_update();
                if now != seen || changes.changed().await.is_err() {
                    return now;
                }
            }
        })
    }

    /// Records that the person saved the recovery key shown last.
    pub(crate) fn confirm_recovery_key(&self, hub: &HubId) -> Result<(), HubError> {
        let slot = self.accounts.slot(hub)?;
        slot.update_cache(|cache| cache.recovery_unconfirmed = false)
            .map_err(credentials)
    }

    /// Issues a new recovery key and replaces the one the hub holds a wrapped copy of. The open
    /// keys are what allows it. The key is marked unconfirmed first, so that a window closed or
    /// a crash before it is saved offers another; it is shown once and stored nowhere.
    pub(crate) async fn reissue_recovery_key(&self, hub: &HubId) -> Result<Secret, HubError> {
        let slot = self.accounts.slot(hub)?;
        let keys = slot.open().ok_or(HubError::KeysLocked)?;
        let client = self.pool.fresh(hub).await?;
        self.reissue_with(&slot, &client, keys.keyset()).await
    }

    async fn reissue_with(
        &self,
        slot: &Slot,
        client: &dyn KeysetHub,
        keyset: &Keyset,
    ) -> Result<Secret, HubError> {
        let hub = slot.hub();
        let before = slot.recovery_unconfirmed(keyset.account_id());
        slot.update_cache(|cache| cache.recovery_unconfirmed = true)
            .map_err(credentials)?;
        let recovery = RecoveryKey::generate().map_err(remote)?;
        let written = self.reseal(hub, client, keyset, &recovery).await;
        match written {
            Ok(record) => {
                if let Err(error) = slot.update_cache(|cache| {
                    cache.keyset = KeysetCopy::of(&record);
                }) {
                    tracing::warn!(%error, hub = %hub, "the account cache cannot be written");
                }
                Ok(Secret::new(recovery.display().as_str()))
            }
            Err(error) => {
                if let Err(error) = slot.update_cache(|cache| cache.recovery_unconfirmed = before) {
                    tracing::warn!(%error, hub = %hub, "the account cache cannot be written");
                }
                Err(error)
            }
        }
    }

    /// Writes the recovery envelope conditional on the keyset version read just before. A
    /// device that wrote the keyset in between (another sign-in, a recovery) makes it conflict:
    /// the record is read again and the write repeated, a few times.
    async fn reseal(
        &self,
        hub: &HubId,
        client: &dyn KeysetHub,
        keyset: &Keyset,
        recovery: &RecoveryKey,
    ) -> Result<v1::AccountKeyset, HubError> {
        let mut attempt = 1;
        loop {
            match client.reseal_recovery(keyset, recovery).await {
                Ok(record) => return Ok(record),
                Err(KeysetError::Hub(error))
                    if error.is_conflict() && attempt < RESEAL_ATTEMPTS =>
                {
                    attempt += 1;
                }
                Err(error) => return Err(self.keyset_failure(hub, error)),
            }
        }
    }

    /// Opens, or creates, the keys with the vault passphrase of an account without a password.
    pub(crate) async fn passphrase(
        &self,
        hub: &HubId,
        passphrase: Secret,
    ) -> Result<KeyStatus, HubError> {
        let client = self.pool.fresh(hub).await?;
        let account = who_am_i(&client, hub, &self.pool).await?;
        self.open_with_passphrase(hub, &client, account.id.as_str(), passphrase)
            .await
    }

    async fn open_with_passphrase(
        &self,
        hub: &HubId,
        client: &dyn KeysetHub,
        account_id: &str,
        passphrase: Secret,
    ) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        let record = match self.record(hub, client).await? {
            Some(record) => record,
            None => {
                let kdf = UnlockKdf::random().map_err(remote)?;
                let unlock = derive(&passphrase, &kdf).await?;
                let method = UnlockMethod::Passphrase;
                if let Some(created) = self
                    .try_create(&slot, client, account_id, &method, &unlock, &kdf)
                    .await?
                {
                    return Ok(created);
                }
                // Another device made the keyset first, derived with a salt of its own: the
                // passphrase is tried again against the keyset that exists.
                self.record(hub, client)
                    .await?
                    .ok_or_else(|| damaged(KeysetError::Incomplete))?
            }
        };
        let kdf = UnlockKdf::parse(&record.kdf, &record.salt)
            .map_err(|error| damaged(KeysetError::Kdf(error)))?;
        let unlock = derive(&passphrase, &kdf).await?;
        match Keyset::open(&record, account_id, &unlock) {
            Err(KeysetError::WrongUnlockKey) => {
                // Nothing says whether the passphrase was mistyped or is a new one: the
                // recovery key decides, and then the typed passphrase is kept.
                let fresh = UnlockKdf::random().map_err(remote)?;
                let unlock = derive(&passphrase, &fresh).await?;
                slot.await_recovery(Recovery {
                    account_id: account_id.to_owned(),
                    unlock,
                    kdf: fresh,
                    method: UnlockMethod::Passphrase,
                });
                Ok(KeyStatus::RecoveryNeeded)
            }
            other => {
                let keyset = other.map_err(|error| self.keyset_failure(hub, error))?;
                let unconfirmed = slot.recovery_unconfirmed(keyset.account_id());
                self.keep(&slot, client, keyset, UnlockMethod::Passphrase, unconfirmed)
                    .await?;
                Ok(opened(&slot, account_id))
            }
        }
    }

    /// Opens the keys with the recovery key, and makes the password typed at sign-in (or the
    /// passphrase) open them from now on.
    pub(crate) async fn recover(&self, hub: &HubId, recovery_key: Secret) -> Result<(), HubError> {
        let client = self.pool.fresh(hub).await?;
        self.recover_with(hub, &client, recovery_key).await
    }

    async fn recover_with(
        &self,
        hub: &HubId,
        client: &dyn KeysetHub,
        recovery_key: Secret,
    ) -> Result<(), HubError> {
        let slot = self.accounts.slot(hub)?;
        let pending = slot.recovery().ok_or(HubError::NoRecoveryPending)?;
        let key =
            RecoveryKey::parse(recovery_key.expose()).map_err(|_| HubError::RecoveryKeyInvalid)?;
        let keyset = client
            .recover(&pending.account_id, &key, &pending.unlock, &pending.kdf)
            .await
            .map_err(|error| match error {
                KeysetError::WrongRecoveryKey => HubError::RecoveryKeyInvalid,
                other => self.keyset_failure(hub, other),
            })?;
        // The person typed the recovery key, so it is the one they hold.
        self.keep(&slot, client, keyset, pending.method.clone(), false)
            .await
    }

    async fn settle(
        &self,
        slot: &Slot,
        client: &dyn KeysetHub,
        account_id: &str,
        method: UnlockMethod,
        unlock: UnlockKey,
        kdf: UnlockKdf,
    ) -> Result<KeyStatus, HubError> {
        let hub = slot.hub();
        let record = match self.record(hub, client).await? {
            Some(record) => record,
            None => {
                let created = self
                    .try_create(slot, client, account_id, &method, &unlock, &kdf)
                    .await?;
                if let Some(created) = created {
                    return Ok(created);
                }
                // Another device made the keyset first: this one opens it, never replaces it.
                self.record(hub, client)
                    .await?
                    .ok_or_else(|| damaged(KeysetError::Incomplete))?
            }
        };
        match Keyset::open(&record, account_id, &unlock) {
            Ok(keyset) => {
                let unconfirmed = slot.recovery_unconfirmed(keyset.account_id());
                self.keep(slot, client, keyset, method, unconfirmed).await?;
                Ok(opened(slot, account_id))
            }
            Err(KeysetError::WrongUnlockKey) => {
                slot.await_recovery(Recovery {
                    account_id: account_id.to_owned(),
                    unlock,
                    kdf,
                    method,
                });
                Ok(KeyStatus::RecoveryNeeded)
            }
            Err(error) => Err(self.keyset_failure(hub, error)),
        }
    }

    /// Creates the keyset and opens the vault with it. `None` when another device created the
    /// keyset first.
    async fn try_create(
        &self,
        slot: &Slot,
        client: &dyn KeysetHub,
        account_id: &str,
        method: &UnlockMethod,
        unlock: &UnlockKey,
        kdf: &UnlockKdf,
    ) -> Result<Option<KeyStatus>, HubError> {
        let hub = slot.hub();
        let made = creation(client.create(account_id, unlock, kdf).await)
            .map_err(|error| self.keyset_failure(hub, error))?;
        match made {
            Creation::Created(new) => {
                // Marked before the key is shown: if it is never confirmed, the next time the
                // keys open another is offered.
                self.keep(slot, client, new.keyset, method.clone(), true)
                    .await?;
                Ok(Some(KeyStatus::Created(Secret::new(
                    new.recovery.display().as_str(),
                ))))
            }
            Creation::Raced => Ok(None),
        }
    }

    /// Opens the vault with `keyset` and writes the cache from the record the hub now holds.
    async fn keep(
        &self,
        slot: &Slot,
        client: &dyn KeysetHub,
        keyset: Keyset,
        method: UnlockMethod,
        unconfirmed: bool,
    ) -> Result<(), HubError> {
        let hub = slot.hub();
        let record = self
            .record(hub, client)
            .await?
            .ok_or_else(|| damaged(KeysetError::Incomplete))?;
        let mut cache = AccountCache::new(
            client.hub_uid(),
            keyset.account_id().to_owned(),
            method,
            &record,
        );
        cache.recovery_unconfirmed = unconfirmed;
        slot.install(OpenKeys::new(keyset), cache);
        Ok(())
    }

    async fn record(
        &self,
        hub: &HubId,
        client: &dyn KeysetHub,
    ) -> Result<Option<v1::AccountKeyset>, HubError> {
        client
            .record()
            .await
            .map_err(|error| self.pool.failed(hub, error))
    }

    fn keyset_failure(&self, hub: &HubId, error: KeysetError) -> HubError {
        match error {
            KeysetError::Hub(error) => self.pool.failed(hub, error),
            other => keyset_error(other),
        }
    }
}

/// The status of keys that are open: an unconfirmed recovery key is offered again.
fn opened(slot: &Slot, account_id: &str) -> KeyStatus {
    if slot.recovery_unconfirmed(account_id) {
        KeyStatus::RecoveryKeyUnconfirmed
    } else {
        KeyStatus::Open
    }
}

/// A keyset failure that is not a hub call failing.
fn keyset_error(error: KeysetError) -> HubError {
    match error {
        KeysetError::Damaged(_)
        | KeysetError::Kdf(_)
        | KeysetError::Incomplete
        | KeysetError::PublicKey(_)
        | KeysetError::AlreadyExists
        | KeysetError::WrongRecoveryKey
        | KeysetError::WrongUnlockKey => damaged(error),
        other => remote(other),
    }
}

fn damaged(error: KeysetError) -> HubError {
    HubError::KeysetDamaged {
        cause: SharedCause::new(error),
    }
}

/// The unlock key of a vault passphrase. Key derivation is slow and memory hard, so it does not
/// run on a thread that serves other hub calls.
async fn derive(passphrase: &Secret, kdf: &UnlockKdf) -> Result<UnlockKey, HubError> {
    let (passphrase, kdf) = (passphrase.clone(), kdf.clone());
    let derived = tokio::task::spawn_blocking(move || {
        UnlockKey::from_vault_password(passphrase.expose(), &kdf)
    })
    .await
    .map_err(remote)?;
    derived.map_err(|error| match error {
        LoginError::PasswordTooShort { .. } | LoginError::EmptyUsername => {
            HubError::InvalidVaultPassphrase
        }
        other => remote(other),
    })
}

#[cfg(test)]
mod tests;
