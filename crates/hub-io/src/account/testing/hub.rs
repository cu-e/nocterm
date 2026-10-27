//! The keyset of one account on a pretend hub that several devices share.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use nocterm_hub_client::{ClientError, Keyset, KeysetError, NewKeyset};
use nocterm_hub_proto::v1;
use nocterm_keys::{AccountKey, AccountKeyPair, RecoveryKey, UnlockKdf, UnlockKey};
use tokio::sync::Barrier;

use super::{HUB_UID, sealed_record};
use crate::account::keyset_hub::KeysetHub;

#[derive(Default)]
struct Held {
    record: Option<v1::AccountKeyset>,
    /// Devices that read "no keyset" wait here for each other before they go on.
    meet: Option<Arc<Barrier>>,
    creations: usize,
}

/// The keyset a hub holds for one account. Clones are devices of the same account: they share
/// what the hub holds.
#[derive(Clone, Default)]
pub(crate) struct MemoryKeysets {
    held: Arc<Mutex<Held>>,
}

impl MemoryKeysets {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The record the hub holds.
    pub(crate) fn stored(&self) -> Option<v1::AccountKeyset> {
        self.held().record.clone()
    }

    /// Makes the first `devices` that find no keyset wait for each other before they create it,
    /// so all of them try at once.
    pub(crate) fn meet_before_creating(&self, devices: usize) {
        self.held().meet = Some(Arc::new(Barrier::new(devices)));
    }

    /// How many devices tried to create the keyset.
    pub(crate) fn creations(&self) -> usize {
        self.held().creations
    }
}

impl KeysetHub for MemoryKeysets {
    fn hub_uid(&self) -> String {
        HUB_UID.to_owned()
    }

    fn record(&self) -> BoxFuture<'_, Result<Option<v1::AccountKeyset>, ClientError>> {
        Box::pin(async move {
            let (seen, meet) = {
                let held = self.held();
                (held.record.clone(), held.meet.clone())
            };
            if let (None, Some(meet)) = (&seen, meet) {
                meet.wait().await;
            }
            Ok(seen)
        })
    }

    fn create<'a>(
        &'a self,
        account_id: &'a str,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<NewKeyset, KeysetError>> {
        Box::pin(async move {
            let mut held = self.held();
            held.creations += 1;
            if held.record.is_some() {
                return Err(KeysetError::AlreadyExists);
            }
            let account_key = AccountKey::generate()?;
            let pair = AccountKeyPair::generate()?;
            let recovery = RecoveryKey::generate()?;
            let mut record = sealed_record(account_id, unlock, kdf, &account_key, &pair, &recovery);
            record.version = 1;
            let keyset = Keyset::open(&record, account_id, unlock)?;
            held.record = Some(record);
            Ok(NewKeyset { keyset, recovery })
        })
    }

    fn recover<'a>(
        &'a self,
        account_id: &'a str,
        recovery: &'a RecoveryKey,
        unlock: &'a UnlockKey,
        kdf: &'a UnlockKdf,
    ) -> BoxFuture<'a, Result<Keyset, KeysetError>> {
        Box::pin(async move {
            let mut held = self.held();
            let record = held.record.clone().ok_or(KeysetError::Incomplete)?;
            let keyset = Keyset::recover(&record, account_id, recovery)?;
            let mut next = sealed_record(
                account_id,
                unlock,
                kdf,
                keyset.account_key(),
                keyset.key_pair(),
                recovery,
            );
            next.version = record.version + 1;
            let opened = Keyset::open(&next, account_id, unlock)?;
            held.record = Some(next);
            Ok(opened)
        })
    }
}
