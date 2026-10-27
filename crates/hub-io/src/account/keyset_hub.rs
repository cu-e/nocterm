//! The keyset calls of a sign-in, as one narrow port to a hub.
//!
//! [`KeyFlow`](super::KeyFlow) decides what to do with the account's keys; this port is only
//! what it asks of the hub. [`HubClient`] implements it over the contract; tests implement it
//! over memory, so a race between two devices needs no network.

use futures::future::BoxFuture;
use nocterm_hub_client::{ClientError, HubClient, Keyset, KeysetError, NewKeyset};
use nocterm_hub_proto::v1;
use nocterm_keys::{RecoveryKey, UnlockKdf, UnlockKey};

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
}
