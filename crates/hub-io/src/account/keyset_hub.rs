//! The keyset calls of a sign-in, as one narrow port to a hub.
//!
//! [`KeyFlow`](super::KeyFlow) decides what to do with the account's keys; this port is only
//! what it asks of the hub. [`HubClient`] implements it over the contract; tests implement it
//! over memory, so a race between two devices needs no network.

use futures::future::BoxFuture;
use nocterm_hub_client::{ClientError, HubClient, Keyset, KeysetError, NewKeyset};
use nocterm_hub_proto::v1;
use nocterm_keys::{OpenError, RecoveryKey, UnlockKdf, UnlockKey};

/// What a key flow asks of one hub.
pub(crate) trait KeysetHub: Send + Sync {
    /// The hub's own identifier, which the login key is derived with.
    fn hub_uid(&self) -> String;

    /// The account's keyset record; `None` before a device created it.
    fn record(&self) -> BoxFuture<'_, Result<Option<v1::AccountKeyset>, ClientError>>;

    /// Creates the keyset, conditional on there being none: a device that lost the race gets
    /// [`KeysetError::AlreadyExists`].
    fn create<'a>(
        &'a self,
        account_id: &'a str,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<NewKeyset, KeysetError>>;

    /// Opens the keyset with the recovery key and seals the account key anew under `unlock`.
    fn recover<'a>(
        &'a self,
        account_id: &'a str,
        recovery: &'a RecoveryKey,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<Keyset, KeysetError>>;

    /// Seals the account key of `keyset`, which is open, under `recovery` in place of the
    /// recovery envelope the hub holds. One write conditional on the version just read: a
    /// version that moved in between is a conflict ([`ClientError::is_conflict`]) the caller
    /// retries. Returns the record the hub now holds.
    fn reseal_recovery<'a>(
        &'a self,
        keyset: &'a Keyset,
        recovery: &'a RecoveryKey,
    ) -> BoxFuture<'a, Result<v1::AccountKeyset, KeysetError>>;
}

/// The write that replaces the recovery envelope of `record` and nothing else. The other values
/// are the hub's own, and a record whose public key is not the one of the open `keyset` is not
/// this account's: it is refused instead of sealed over.
pub(crate) fn resealed(
    record: &v1::AccountKeyset,
    keyset: &Keyset,
    recovery: &RecoveryKey,
) -> Result<v1::PutAccountKeysetRequest, KeysetError> {
    if record.public_key != keyset.public_key().to_bytes() {
        return Err(KeysetError::Damaged(OpenError::Rejected));
    }
    Ok(v1::PutAccountKeysetRequest {
        expected_version: record.version,
        kdf: record.kdf.clone(),
        salt: record.salt.clone(),
        wrapped_account_key: record.wrapped_account_key.clone(),
        public_key: record.public_key.clone(),
        sealed_private_key: record.sealed_private_key.clone(),
        recovery_wrapped_account_key: recovery.wrap(keyset.account_key(), keyset.account_id())?,
        unlock_kind: record.unlock_kind,
    })
}

impl KeysetHub for HubClient {
    fn hub_uid(&self) -> String {
        self.info().hub_id.clone()
    }

    fn record(&self) -> BoxFuture<'_, Result<Option<v1::AccountKeyset>, ClientError>> {
        Box::pin(self.keyset_record())
    }

    fn create<'a>(
        &'a self,
        account_id: &'a str,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<NewKeyset, KeysetError>> {
        Box::pin(self.create_keyset(account_id, unlock, kdf))
    }

    fn recover<'a>(
        &'a self,
        account_id: &'a str,
        recovery: &'a RecoveryKey,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<Keyset, KeysetError>> {
        Box::pin(self.recover_keyset(account_id, recovery, unlock, kdf))
    }

    fn reseal_recovery<'a>(
        &'a self,
        keyset: &'a Keyset,
        recovery: &'a RecoveryKey,
    ) -> BoxFuture<'a, Result<v1::AccountKeyset, KeysetError>> {
        Box::pin(reseal(self, keyset, recovery))
    }
}

async fn reseal(
    client: &HubClient,
    keyset: &Keyset,
    recovery: &RecoveryKey,
) -> Result<v1::AccountKeyset, KeysetError> {
    let record = client
        .keyset_record()
        .await?
        .ok_or(KeysetError::Incomplete)?;
    let request = resealed(&record, keyset, recovery)?;
    let stored = client
        .store()
        .put_account_keyset(request)
        .await
        .map_err(ClientError::from)?;
    Ok(stored.into_inner())
}
