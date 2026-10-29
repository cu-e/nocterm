use super::*;
use crate::credentials::CredentialDir;

fn session(account: &str, token: &str) -> Session {
    Session {
        account_id: account.into(),
        device_id: "device".into(),
        access_token: token.into(),
        refresh_token: format!("refresh-{token}"),
        access_expires_at_ms: i64::MAX,
        refresh_expires_at_ms: i64::MAX,
    }
}
fn file(dir: &tempfile::TempDir) -> CredentialFile {
    CredentialDir::new(dir.path().to_path_buf())
        .file(&"hub.test".parse().unwrap())
        .unwrap()
}
fn signed_in(dir: &tempfile::TempDir, session: &Session) -> BoundSessionStore {
    let file = file(dir);
    file.load_or_create().unwrap();
    let login = file.save_session(session).unwrap();
    file.bound(session, Some(login))
}

#[test]
fn a_bound_store_adopts_only_a_refresh_of_the_original_subject() {
    let dir = tempfile::tempdir().unwrap();
    let store = signed_in(&dir, &session("A", "initial"));
    let refreshed = session("A", "new");
    {
        let current = file(&dir);
        let _lock = current.lock().unwrap();
        SessionStore::save(&current, &refreshed).unwrap();
    }
    let _lock = store.lock().unwrap();
    assert_eq!(store.load().unwrap(), Some(refreshed.clone()));
    store.save(&session("A", "next")).unwrap();
    assert_eq!(
        SessionStore::load(&file(&dir)).unwrap(),
        Some(session("A", "next"))
    );
}

#[test]
fn an_old_a_store_cannot_adopt_b_and_a_new_b_store_can_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let old = signed_in(&dir, &session("A", "initial"));
    let current = signed_in(&dir, &session("B", "login"));
    {
        let _lock = old.lock().unwrap();
        let error = old.load().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.get_ref().unwrap().is::<ReplacedLogin>());
    }
    let _lock = current.lock().unwrap();
    assert_eq!(current.load().unwrap(), Some(session("B", "login")));
    current.save(&session("B", "refreshed")).unwrap();
    assert_eq!(
        SessionStore::load(&file(&dir)).unwrap(),
        Some(session("B", "refreshed"))
    );
}

#[test]
fn a_late_a_refresh_cannot_overwrite_the_file_of_b() {
    let dir = tempfile::tempdir().unwrap();
    let old = signed_in(&dir, &session("A", "initial"));
    file(&dir).save_session(&session("B", "login")).unwrap();
    let _lock = old.lock().unwrap();
    assert_eq!(
        old.save(&session("A", "late")).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        SessionStore::load(&file(&dir)).unwrap(),
        Some(session("B", "login"))
    );
}

#[test]
fn a_cleared_login_cannot_implicitly_refresh_old_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let old = signed_in(&dir, &session("A", "initial"));
    file(&dir).clear_session().unwrap();
    let _lock = old.lock().unwrap();
    assert!(old.load().is_err());
    assert!(old.save(&session("A", "late")).is_err());
    assert!(SessionStore::load(&file(&dir)).unwrap().is_none());
}

#[test]
fn a_changed_device_is_a_replacement_even_for_the_same_account() {
    let dir = tempfile::tempdir().unwrap();
    let old = signed_in(&dir, &session("A", "initial"));
    let mut next = session("A", "login");
    next.device_id = "new-device".into();
    file(&dir).save_session(&next).unwrap();
    let _lock = old.lock().unwrap();
    assert!(old.load().is_err());
    assert!(old.save(&session("A", "late")).is_err());
    assert_eq!(SessionStore::load(&file(&dir)).unwrap(), Some(next));
}

#[test]
fn captured_revocation_erases_only_the_matching_file_subject() {
    let dir = tempfile::tempdir().unwrap();
    let _old = signed_in(&dir, &session("A", "initial"));
    file(&dir).save_session(&session("B", "login")).unwrap();
    file(&dir).erase_if("A", "device", _old.login).unwrap();
    assert_eq!(
        SessionStore::load(&file(&dir)).unwrap(),
        Some(session("B", "login"))
    );
    file(&dir)
        .erase_if("B", "device", file(&dir).load().unwrap().unwrap().login)
        .unwrap();
    assert!(file(&dir).load().unwrap().is_none());
}

#[test]
fn a_same_subject_new_login_rejects_old_adoption_refresh_and_revocation_before_pool_publication() {
    let dir = tempfile::tempdir().unwrap();
    let old = signed_in(&dir, &session("A", "old-login"));
    // This is the exact formerly vulnerable window: the new file is durable,
    // while a retained old membership can still deliver its delayed failure.
    let next = session("A", "new-login");
    let new_login = file(&dir).save_session(&next).unwrap();
    assert_ne!(old.login, Some(new_login));
    assert!(!file(&dir).matches_login("A", "device", old.login).unwrap());
    assert!(
        file(&dir)
            .matches_login("A", "device", Some(new_login))
            .unwrap()
    );
    file(&dir).erase_if("A", "device", old.login).unwrap();
    {
        let _lock = old.lock().unwrap();
        assert_eq!(
            old.load().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            old.save(&session("A", "late-refresh")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    assert_eq!(SessionStore::load(&file(&dir)).unwrap(), Some(next.clone()));
    let current = file(&dir).bound(&next, Some(new_login));
    let _lock = current.lock().unwrap();
    current.save(&session("A", "current-refresh")).unwrap();
    assert_eq!(
        SessionStore::load(&file(&dir)).unwrap(),
        Some(session("A", "current-refresh"))
    );
    assert_eq!(file(&dir).load().unwrap().unwrap().login, Some(new_login));
}

#[test]
fn a_legacy_login_cannot_retire_or_adopt_a_new_explicit_login() {
    let dir = tempfile::tempdir().unwrap();
    let original = session("A", "legacy");
    file(&dir).load_or_create().unwrap();
    SessionStore::save(&file(&dir), &original).unwrap();
    let old = file(&dir).bound(&original, None);
    let next = session("A", "new-login");
    file(&dir).save_session(&next).unwrap();
    file(&dir).erase_if("A", "device", None).unwrap();
    let _lock = old.lock().unwrap();
    assert!(old.load().is_err());
    assert!(old.save(&session("A", "late")).is_err());
    assert_eq!(SessionStore::load(&file(&dir)).unwrap(), Some(next));
}

#[test]
fn late_original_revocation_preserves_new_same_account_file_and_open_slot() {
    use crate::account::{
        Accounts,
        testing::{ACCOUNT_ID, hub, sign_in},
    };
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let accounts = Accounts::new(dir.path().to_path_buf());
    let (slot, _) = sign_in(&accounts);
    let original_keys = slot.open().unwrap();
    let old = signed_in(&dir, &session(ACCOUNT_ID, "original"));
    // Publish the replacement credentials before the replacement membership.
    let next = session(ACCOUNT_ID, "new-login");
    file(&dir).save_session(&next).unwrap();
    let (new_slot, _) = sign_in(&accounts);
    let current_keys = new_slot.open().unwrap();
    file(&dir)
        .erase_if(ACCOUNT_ID, "device", old.login)
        .unwrap();
    slot.erase_if(&original_keys).unwrap();
    assert_eq!(SessionStore::load(&file(&dir)).unwrap(), Some(next));
    assert!(Arc::ptr_eq(
        &accounts.slot(&hub()).unwrap().open().unwrap(),
        &current_keys
    ));
}
