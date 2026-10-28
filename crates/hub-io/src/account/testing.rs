//! Keys for tests, made the way a first sign-in makes them but without a hub.

use std::sync::Arc;

use nocterm_hub_client::Keyset;
use nocterm_hub_proto::v1;
use nocterm_hubs_model::HubId;
use nocterm_keys::{AccountKey, AccountKeyPair, KdfParams, RecoveryKey, UnlockKdf, UnlockKey};

use super::{Accounts, OpenKeys, Slot};
use crate::cache::{AccountCache, UnlockMethod};

mod hub;

pub(crate) use hub::MemoryKeysets;

pub(crate) const ACCOUNT_ID: &str = "account-1";
pub(crate) const HUB_UID: &str = "hub-uid";
pub(crate) const PASSWORD: &str = "correct horse battery";

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
    pair: &AccountKeyPair,
    recovery: &RecoveryKey,
) -> v1::AccountKeyset {
    v1::AccountKeyset {
        version: 0,
        kdf: kdf.description(),
        salt: kdf.salt().to_vec(),
        wrapped_account_key: account_key.seal(unlock, account_id).unwrap(),
        updated_at_ms: 0,
        public_key: pair.public_key().to_bytes().to_vec(),
        sealed_private_key: pair.seal(account_key, account_id).unwrap(),
        recovery_wrapped_account_key: recovery.wrap(account_key, account_id).unwrap(),
    }
}

/// A keyset record sealed under `unlock`, and the recovery key that also opens it.
pub(crate) fn record(unlock: &UnlockKey) -> (v1::AccountKeyset, RecoveryKey) {
    let account_key = AccountKey::generate().unwrap();
    let pair = AccountKeyPair::generate().unwrap();
    let recovery = RecoveryKey::generate().unwrap();
    let mut record = sealed_record(ACCOUNT_ID, unlock, &kdf(), &account_key, &pair, &recovery);
    record.version = 1;
    (record, recovery)
}

pub(crate) fn open(record: &v1::AccountKeyset, unlock: &UnlockKey) -> OpenKeys {
    OpenKeys::new(Keyset::open(record, ACCOUNT_ID, unlock).expect("the record opens"))
}

pub(crate) fn cache(record: &v1::AccountKeyset, method: UnlockMethod) -> AccountCache {
    AccountCache::new(HUB_UID.into(), ACCOUNT_ID.into(), method, record)
}

/// Open keys on their own, for code that only needs to seal and open collections.
pub(crate) fn open_keys() -> Arc<OpenKeys> {
    let unlock = unlock(PASSWORD);
    let (record, _) = record(&unlock);
    Arc::new(open(&record, &unlock))
}

/// Open keys with the record they were opened from, for tests that grant the account a key.
pub(crate) fn open_keys_with_record() -> (Arc<OpenKeys>, v1::AccountKeyset) {
    let unlock = unlock(PASSWORD);
    let (record, _) = record(&unlock);
    (Arc::new(open(&record, &unlock)), record)
}

/// A collection key of an organization and the envelope an administrator would store for the
/// account: the key sealed to the account's published public key.
pub(crate) fn granted(
    record: &v1::AccountKeyset,
    collection_id: &str,
) -> (nocterm_keys::CollectionKey, Vec<u8>) {
    let key = nocterm_keys::CollectionKey::generate().expect("a fresh key");
    let public = nocterm_keys::PublicKey::from_bytes(&record.public_key).expect("a public key");
    let envelope = key
        .seal_to(&public, collection_id, ACCOUNT_ID)
        .expect("the grant is sealed");
    (key, envelope)
}

pub(crate) fn hub() -> HubId {
    HubId::from(
        "hub.test"
            .parse::<nocterm_hubs_model::HubAddress>()
            .unwrap(),
    )
}

/// The slot of the test hub in `accounts`, opened as a sign-in with `PASSWORD` opens it.
pub(crate) fn sign_in(accounts: &Accounts) -> (Arc<Slot>, v1::AccountKeyset) {
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
pub(crate) fn signed_in(dir: &std::path::Path) -> (Accounts, Arc<Slot>, v1::AccountKeyset) {
    let accounts = Accounts::new(dir.to_path_buf());
    let (slot, record) = sign_in(&accounts);
    (accounts, slot, record)
}

/// Another device of the same account: the same keyset, opened with the same password.
pub(crate) fn second_device(
    dir: &std::path::Path,
    record: &v1::AccountKeyset,
) -> (Accounts, Arc<Slot>) {
    let accounts = Accounts::new(dir.to_path_buf());
    let slot = accounts.slot(&hub()).unwrap();
    let unlock = unlock(PASSWORD);
    slot.install(
        open(record, &unlock),
        cache(record, UnlockMethod::Passphrase),
    );
    (accounts, slot)
}
