//! Keys for tests, made the way a first sign-in makes them but without a hub.

use std::sync::Arc;

use nocterm_hub_client::Keyset;
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubId;
use nocterm_keys::{AccountKey, KdfParams, RecoveryKey, UnlockKdf, UnlockKey, UnlockSource};

use super::{Accounts, OpenKeys, Slot};
use crate::cache::{AccountCache, UnlockMethod};

mod accounts;
mod hub;
mod native_operations;
mod subjects;
pub(crate) use accounts::NativeAccounts;
pub(crate) use native_operations::{NativeVault, native_operations};

pub(crate) use hub::MemoryKeysets;
pub(crate) use subjects::ScriptedSubjects;

pub(crate) const ACCOUNT_ID: &str = "account-1";
pub(crate) const HUB_UID: &str = "hub-uid";
pub(crate) const PASSWORD: &str = "correct horse battery";

/// Occupies the existing physical cache worker until the shared gate is released.
/// Polling its returned future controls FIFO admission without an I/O injection hook.
pub(crate) struct CacheJobRelease(nocterm_session::testing::Gate);
impl CacheJobRelease {
    pub(crate) fn release(&self) {
        self.0.release();
    }
}
impl Drop for CacheJobRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

pub(crate) fn blocked_cache_job(
    slot: &Slot,
) -> (
    futures::future::BoxFuture<'static, Result<(), crate::cache::CacheError>>,
    nocterm_session::testing::Gate,
    CacheJobRelease,
) {
    let entered = nocterm_session::testing::Gate::pending();
    let release = nocterm_session::testing::Gate::pending();
    let worker_entered = entered.clone();
    let worker_release = release.clone();
    let work = slot.cache_work().clone();
    let job = Box::pin(async move {
        work.run(move || {
            worker_entered.release();
            futures::executor::block_on(worker_release.wait());
            Ok(())
        })
        .await
    });
    (job, entered, CacheJobRelease(release))
}

pub(crate) fn kdf() -> UnlockKdf {
    UnlockKdf::new(KdfParams::MINIMUM, vec![7; 16]).expect("a valid derivation")
}

pub(crate) fn unlock(password: &str) -> UnlockKey {
    UnlockKey::from_vault_password(password, &kdf()).expect("a usable password")
}

/// A keyset record, as a first sign-in seals it, at version 0.
pub(crate) fn sealed_record(
    account_id: &str,
    unlock: &UnlockKey,
    kdf: &UnlockKdf,
    account_key: &AccountKey,
    recovery: &RecoveryKey,
) -> v2::Keyset {
    v2::Keyset {
        version: 0,
        kdf: kdf.description(),
        salt: kdf.salt().to_vec(),
        wrapped_account_key: account_key.seal(unlock, account_id).unwrap(),
        updated_at_ms: 0,
        recovery_wrapped_account_key: recovery.wrap(account_key, account_id).unwrap(),
        unlock_kind: match unlock.source() {
            UnlockSource::LoginPassword => v2::UnlockKind::LoginPassword,
            UnlockSource::VaultPassword => v2::UnlockKind::VaultPassword,
        } as i32,
    }
}

/// A keyset record sealed under `unlock`, and the recovery key that also opens it.
pub(crate) fn record(unlock: &UnlockKey) -> (v2::Keyset, RecoveryKey) {
    let account_key = AccountKey::generate().unwrap();
    let recovery = RecoveryKey::generate().unwrap();
    let mut record = sealed_record(ACCOUNT_ID, unlock, &kdf(), &account_key, &recovery);
    record.version = 1;
    (record, recovery)
}

pub(crate) fn open(record: &v2::Keyset, unlock: &UnlockKey) -> OpenKeys {
    OpenKeys::new(Keyset::open(record, ACCOUNT_ID, unlock).expect("the record opens"))
}

pub(crate) fn cache(record: &v2::Keyset, method: UnlockMethod) -> AccountCache {
    AccountCache::new(HUB_UID.into(), ACCOUNT_ID.into(), method, record)
}

/// Open keys on their own, for code that only needs to seal and open collections.
pub(crate) fn open_keys() -> Arc<OpenKeys> {
    let unlock = unlock(PASSWORD);
    let (record, _) = record(&unlock);
    Arc::new(open(&record, &unlock))
}

pub(crate) fn hub() -> HubId {
    hub_for("hub.test")
}

pub(crate) fn hub_for(address: &str) -> HubId {
    HubId::from(address.parse::<nocterm_hubs_model::HubAddress>().unwrap())
}

/// The slot of the test hub in `accounts`, opened as a sign-in with `PASSWORD` opens it.
pub(crate) fn sign_in(accounts: &Accounts) -> (Arc<Slot>, v2::Keyset) {
    let slot = accounts.slot(&hub()).unwrap();
    let unlock = unlock(PASSWORD);
    let (record, _) = record(&unlock);
    slot.install(
        open(&record, &unlock),
        cache(&record, UnlockMethod::Passphrase),
    );
    (slot, record)
}

/// A slot whose vault was opened by a sign-in with `PASSWORD`, kept in `dir`.
pub(crate) fn signed_in(dir: &std::path::Path) -> (Accounts, Arc<Slot>, v2::Keyset) {
    let accounts = Accounts::new(dir.to_path_buf());
    let (slot, record) = sign_in(&accounts);
    (accounts, slot, record)
}

/// Another device of the same account: the same keyset, opened with the same password.
pub(crate) fn second_device(dir: &std::path::Path, record: &v2::Keyset) -> (Accounts, Arc<Slot>) {
    let accounts = Accounts::new(dir.to_path_buf());
    let slot = accounts.slot(&hub()).unwrap();
    let unlock = unlock(PASSWORD);
    slot.install(
        open(record, &unlock),
        cache(record, UnlockMethod::Passphrase),
    );
    (accounts, slot)
}
