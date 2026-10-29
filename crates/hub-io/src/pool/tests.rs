//! Offline/reconnect failures retire the captured native file and Slot receipt.
use super::*;
use crate::{
    account::testing::{ACCOUNT_ID, hub, sign_in},
    cache::CacheFile,
    pool::observation::LoginObservation,
};
use nocterm_hub_client::Session;
use nocterm_vault::VaultStatus;

struct Login {
    directory: tempfile::TempDir,
    accounts: Arc<Accounts>,
    pool: Arc<Pool>,
    file: CredentialFile,
    slot: Arc<Slot>,
    session: Session,
    login: CredentialLoginId,
    observed: Arc<LoginObservation>,
}
impl Login {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let accounts = Arc::new(Accounts::new(directory.path().to_path_buf()));
        let (slot, _) = sign_in(&accounts);
        let pool = Arc::new(Pool::new(
            directory.path().to_path_buf(),
            "test",
            accounts.clone(),
        ));
        let file = pool.file(hub().address()).unwrap();
        file.load_or_create().unwrap();
        let session = Session {
            account_id: ACCOUNT_ID.into(),
            device_id: "device".into(),
            access_token: "access-original".into(),
            refresh_token: "refresh-original".into(),
            access_expires_at_ms: i64::MAX,
            refresh_expires_at_ms: i64::MAX,
        };
        let login = file.save_session(&session).unwrap();
        let observed = LoginObservation::capture(pool.clone(), hub()).unwrap();
        Self {
            directory,
            accounts,
            pool,
            file,
            slot,
            session,
            login,
            observed,
        }
    }
    fn cached(&self) -> bool {
        CacheFile::new(self.directory.path(), hub().address())
            .unwrap()
            .load()
            .unwrap()
            .is_some()
    }
    fn retire(&self, erase: bool) -> Result<(), HubError> {
        self.observed.retire(
            &self.file,
            Some((&self.session.account_id, &self.session.device_id)),
            Some(self.login),
            erase,
        )
    }
}

#[test]
fn a_captured_revocation_erases_only_its_original_native_login_keys_and_cache() {
    let login = Login::new();
    assert!(login.cached());
    login.retire(true).unwrap();
    assert!(login.slot.open().is_none());
    assert_eq!(login.slot.status(), VaultStatus::Missing);
    assert!(!login.cached());
    assert!(login.file.load().unwrap().is_none());
}

#[test]
fn a_captured_revocation_preserves_the_keys_and_cache_of_another_hub() {
    let login = Login::new();
    let other = HubId::from(HubAddress::parse("other.test").unwrap());
    let file = login.pool.file(other.address()).unwrap();
    file.load_or_create().unwrap();
    let mut session = login.session.clone();
    session.account_id = "other-account".into();
    let id = file.save_session(&session).unwrap();
    let observed = LoginObservation::capture(login.pool.clone(), other).unwrap();
    observed
        .retire(
            &file,
            Some((&session.account_id, &session.device_id)),
            Some(id),
            true,
        )
        .unwrap();
    assert_eq!(login.slot.status(), VaultStatus::Unlocked);
    assert!(login.cached());
    assert_eq!(
        login.file.load().unwrap().unwrap().session,
        Some(login.session)
    );
}

#[test]
fn captured_expiry_closes_keys_but_keeps_the_device_key_and_ciphertext_for_sign_in() {
    let login = Login::new();
    let public_key = login.file.load().unwrap().unwrap().key.public_key();
    login.retire(false).unwrap();
    assert!(login.slot.open().is_none());
    assert_eq!(login.slot.status(), VaultStatus::Locked);
    assert!(login.cached());
    let device = login.file.load().unwrap().unwrap();
    assert_eq!(device.key.public_key(), public_key);
    assert!(device.session.is_none());
}

#[test]
fn a_late_captured_revocation_cannot_erase_a_new_login_of_the_same_account() {
    let login = Login::new();
    let mut replacement = login.session.clone();
    replacement.access_token = "access-replacement".into();
    replacement.refresh_token = "refresh-replacement".into();
    let current = login.file.save_session(&replacement).unwrap();
    assert_ne!(current, login.login);
    sign_in(&login.accounts);
    let key = login.slot.open().unwrap().keyset().public_key();
    assert!(matches!(login.retire(true), Err(HubError::Stale)));
    assert_eq!(login.slot.status(), VaultStatus::Unlocked);
    assert_eq!(login.slot.open().unwrap().keyset().public_key(), key);
    assert!(login.cached());
    let device = login.file.load().unwrap().unwrap();
    assert_eq!(device.login, Some(current));
    assert_eq!(device.session, Some(replacement));
}
