//! Opening, creating and recovering the account keys around a sign-in.
//!
//! A first sign-in creates the keyset (one conditional write: a second device that races it
//! opens the keyset that won, it never overwrites it). A later sign-in opens it with the key
//! derived from the password. A password that does not open it (it was reset) leaves the keys
//! waiting for the recovery key. A keyset that does not authenticate although the key is right
//! was altered by the hub: that is an error and nothing is written.

use std::sync::Arc;

use nocterm_core::SharedCause;
use nocterm_hub_client::{HubClient, Keyset, KeysetError};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{Account, HubError, HubId, KeyStatus};
use nocterm_keys::{LoginError, LoginSecrets, RecoveryKey, UnlockKdf, UnlockKey};
use nocterm_session::Secret;

use super::{Accounts, OpenKeys, Recovery, Slot};
use crate::cache::{AccountCache, UnlockMethod};
use crate::classify::remote;
use crate::directory::who_am_i;
use crate::pool::Pool;

/// The key work of a sign-in, for every hub.
pub(crate) struct KeyFlow {
    pool: Arc<Pool>,
    accounts: Arc<Accounts>,
}

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
        client: &HubClient,
        account: &Account,
        username: &str,
        secrets: LoginSecrets,
    ) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        let kdf = secrets.kdf().clone();
        let unlock = secrets.into_unlock();
        let method = UnlockMethod::Login {
            username: username.trim().to_owned(),
        };
        self.settle(&slot, client, account.id.as_str(), method, unlock, kdf)
            .await
    }

    /// The keys after a browser sign-in: nothing opens them until a passphrase is typed.
    pub(crate) fn after_browser(&self, hub: &HubId) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        Ok(match slot.open() {
            Some(_) => KeyStatus::Open,
            None => KeyStatus::PassphraseNeeded,
        })
    }

    /// Opens, or creates, the keys with the vault passphrase of an account without a password.
    pub(crate) async fn passphrase(
        &self,
        hub: &HubId,
        passphrase: Secret,
    ) -> Result<KeyStatus, HubError> {
        let slot = self.accounts.slot(hub)?;
        let client = self.pool.fresh(hub).await?;
        let account = who_am_i(&client, hub, &self.pool).await?;
        let record = self.record(hub, &client).await?;
        let kdf = match &record {
            Some(record) => UnlockKdf::parse(&record.kdf, &record.salt)
                .map_err(|error| damaged(KeysetError::Kdf(error)))?,
            None => UnlockKdf::random().map_err(remote)?,
        };
        let unlock = derive(&passphrase, &kdf).await?;
        match record {
            None => {
                self.create(
                    &slot,
                    &client,
                    account.id.as_str(),
                    UnlockMethod::Passphrase,
                    unlock,
                    kdf,
                )
                .await
            }
            Some(record) => {
                let account_id = account.id.as_str();
                match Keyset::open(&record, account_id, &unlock) {
                    Err(KeysetError::WrongUnlockKey) => {
                        // Nothing says whether the passphrase was mistyped or is a new one:
                        // the recovery key decides, and then the typed passphrase is kept.
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
                        self.keep(&slot, &client, keyset, UnlockMethod::Passphrase)
                            .await?;
                        Ok(KeyStatus::Open)
                    }
                }
            }
        }
    }

    /// Opens the keys with the recovery key, and makes the password typed at sign-in (or the
    /// passphrase) open them from now on.
    pub(crate) async fn recover(&self, hub: &HubId, recovery_key: Secret) -> Result<(), HubError> {
        let slot = self.accounts.slot(hub)?;
        let pending = slot.recovery().ok_or(HubError::NoRecoveryPending)?;
        let key =
            RecoveryKey::parse(recovery_key.expose()).map_err(|_| HubError::RecoveryKeyInvalid)?;
        let client = self.pool.fresh(hub).await?;
        let keyset = client
            .recover_keyset(&pending.account_id, &key, &pending.unlock, &pending.kdf)
            .await
            .map_err(|error| match error {
                KeysetError::WrongRecoveryKey => HubError::RecoveryKeyInvalid,
                other => self.keyset_failure(hub, other),
            })?;
        self.keep(&slot, &client, keyset, pending.method.clone())
            .await
    }

    async fn settle(
        &self,
        slot: &Slot,
        client: &HubClient,
        account_id: &str,
        method: UnlockMethod,
        unlock: UnlockKey,
        kdf: UnlockKdf,
    ) -> Result<KeyStatus, HubError> {
        let hub = slot.hub();
        match self.record(hub, client).await? {
            None => {
                self.create(slot, client, account_id, method, unlock, kdf)
                    .await
            }
            Some(record) => match Keyset::open(&record, account_id, &unlock) {
                Ok(keyset) => {
                    self.keep(slot, client, keyset, method).await?;
                    Ok(KeyStatus::Open)
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
            },
        }
    }

    async fn create(
        &self,
        slot: &Slot,
        client: &HubClient,
        account_id: &str,
        method: UnlockMethod,
        unlock: UnlockKey,
        kdf: UnlockKdf,
    ) -> Result<KeyStatus, HubError> {
        let hub = slot.hub();
        let made = creation(client.create_keyset(account_id, &unlock, &kdf).await)
            .map_err(|error| self.keyset_failure(hub, error))?;
        match made {
            Creation::Created(new) => {
                self.keep(slot, client, new.keyset, method).await?;
                Ok(KeyStatus::Created(Secret::new(
                    new.recovery.display().as_str(),
                )))
            }
            Creation::Raced => {
                let record = self
                    .record(hub, client)
                    .await?
                    .ok_or_else(|| damaged(KeysetError::Incomplete))?;
                match Keyset::open(&record, account_id, &unlock) {
                    Ok(keyset) => {
                        self.keep(slot, client, keyset, method).await?;
                        Ok(KeyStatus::Open)
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
        }
    }

    /// Opens the vault with `keyset` and writes the cache from the record the hub now holds.
    async fn keep(
        &self,
        slot: &Slot,
        client: &HubClient,
        keyset: Keyset,
        method: UnlockMethod,
    ) -> Result<(), HubError> {
        let hub = slot.hub();
        let record = self
            .record(hub, client)
            .await?
            .ok_or_else(|| damaged(KeysetError::Incomplete))?;
        let cache = AccountCache::new(
            client.info().hub_id.clone(),
            keyset.account_id().to_owned(),
            method,
            &record,
        );
        slot.install(OpenKeys::new(keyset), cache);
        Ok(())
    }

    async fn record(
        &self,
        hub: &HubId,
        client: &HubClient,
    ) -> Result<Option<v1::AccountKeyset>, HubError> {
        client
            .keyset_record()
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
