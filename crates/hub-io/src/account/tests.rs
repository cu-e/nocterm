use super::testing::*;
use super::*;
use crate::cache::CacheFile;

#[test]
fn a_signed_in_account_is_unlocked_and_locking_drops_its_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, _) = signed_in(dir.path());
    assert_eq!(slot.status(), VaultStatus::Unlocked);
    let keys = slot.open().unwrap();
    slot.lock();
    assert_eq!(slot.status(), VaultStatus::Locked);
    assert!(slot.open().is_none());
    drop(keys);
}

#[test]
fn locking_and_signing_in_are_announced() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = Accounts::new(dir.path().to_path_buf());
    let slot = accounts.slot(&hub()).unwrap();
    let before = accounts.notice().count();
    let unlock = unlock(PASSWORD);
    let (record, _) = record(&unlock);
    slot.install(
        open(&record, &unlock),
        cache(&record, UnlockMethod::Passphrase),
    );
    assert!(accounts.notice().count() > before);
    let opened = accounts.notice().count();
    slot.lock();
    assert!(accounts.notice().count() > opened);
    let locked = accounts.notice().count();
    slot.lock();
    assert_eq!(
        accounts.notice().count(),
        locked,
        "locking twice changes nothing"
    );
}

#[test]
fn a_new_session_finds_the_account_locked_from_its_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, _slot, _) = signed_in(dir.path());
    let next = Accounts::new(dir.path().to_path_buf());
    let slot = next.slot(&hub()).unwrap();
    assert_eq!(slot.status(), VaultStatus::Locked);
    assert!(slot.read_cache(|cache| cache.is_some()));
}

#[test]
fn an_account_without_a_cache_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = Accounts::new(dir.path().to_path_buf());
    assert_eq!(
        accounts.slot(&hub()).unwrap().status(),
        VaultStatus::Missing
    );
}

#[test]
fn signing_out_erases_the_cache_and_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, slot, _) = signed_in(dir.path());
    slot.erase().unwrap();
    assert_eq!(slot.status(), VaultStatus::Missing);
    let file = CacheFile::new(dir.path(), hub().address()).unwrap();
    assert!(file.load().unwrap().is_none());
    assert_eq!(
        accounts.slot(&hub()).unwrap().status(),
        VaultStatus::Missing
    );
}

#[test]
fn collection_keys_are_separate_and_open_only_their_own_collection() {
    let keys = open_keys();
    let (first, first_envelope) = keys.new_collection("c-1").unwrap();
    let (second, second_envelope) = keys.new_collection("c-2").unwrap();
    let sealed = first.seal_object("c-1", "o", "k", b"secret").unwrap();
    assert!(second.open_object("c-1", "o", "k", &sealed).is_err());
    assert_ne!(first_envelope, second_envelope);
    // An envelope belongs to its collection: it does not open as another one.
    assert!(keys.collection("c-3", &first_envelope).is_err());
    assert!(keys.collection("c-1", &first_envelope).is_ok());
}
