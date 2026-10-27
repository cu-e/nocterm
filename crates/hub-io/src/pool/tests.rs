use nocterm_hubs_model::HubAddress;
use nocterm_vault::VaultStatus;

use super::*;
use crate::account::testing::{hub, sign_in};
use crate::cache::CacheFile;

fn pool(dir: &std::path::Path, accounts: &Arc<Accounts>) -> Pool {
    Pool::new(dir.to_path_buf(), "test", Arc::clone(accounts))
}

fn cached(dir: &std::path::Path, hub: &HubId) -> bool {
    let file = CacheFile::new(dir, hub.address()).unwrap();
    file.load().unwrap().is_some()
}

#[test]
fn a_revoked_device_loses_the_keys_and_the_cached_ciphertext_of_its_hub() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = Arc::new(Accounts::new(dir.path().to_path_buf()));
    let (slot, _) = sign_in(&accounts);
    assert!(cached(dir.path(), &hub()));

    let error = pool(dir.path(), &accounts).failed(&hub(), ClientError::Revoked);

    assert!(matches!(error, HubError::Revoked));
    assert!(slot.open().is_none(), "the keys are wiped");
    assert_eq!(slot.status(), VaultStatus::Missing);
    assert!(!cached(dir.path(), &hub()), "the cache file is erased");
}

#[test]
fn a_revoked_device_keeps_the_account_of_another_hub() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = Arc::new(Accounts::new(dir.path().to_path_buf()));
    let (slot, _) = sign_in(&accounts);
    let other = HubId::from(HubAddress::parse("other.test").unwrap());

    pool(dir.path(), &accounts).failed(&other, ClientError::Revoked);

    assert_eq!(slot.status(), VaultStatus::Unlocked);
    assert!(cached(dir.path(), &hub()));
}

#[test]
fn a_login_the_hub_no_longer_knows_is_not_a_revocation_and_keeps_the_keys_and_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = Arc::new(Accounts::new(dir.path().to_path_buf()));
    let (slot, _) = sign_in(&accounts);

    pool(dir.path(), &accounts).failed(&hub(), ClientError::NotSignedIn);

    assert_eq!(slot.status(), VaultStatus::Unlocked);
    assert!(cached(dir.path(), &hub()));
}
