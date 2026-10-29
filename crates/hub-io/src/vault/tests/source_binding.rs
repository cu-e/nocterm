use super::*;
use crate::account::testing::NativeAccounts;
use crate::store::memory::StoreBoundary;

#[tokio::test]
async fn a_vault_read_queued_before_login_changes_cannot_read_the_replacement_account() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    let pending = fixture.vault().get(id, request_binding());
    fixture.login(1);
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn queued_vault_writes_and_deletes_refuse_a_replacement_login() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    let put = fixture
        .vault()
        .put(None, "A".into(), request_binding(), Secret::new("A"));
    let delete = fixture.vault().delete(id);
    fixture.login(1);
    assert!(matches!(put.await, Err(VaultError::Stale)));
    assert!(matches!(delete.await, Err(VaultError::Stale)));
    assert_eq!(fixture.count(0).await, 1);
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn late_vault_cache_publication_does_not_install_a_records_in_b() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::ChangesReady);
    let pending = fixture.vault().get(id, request_binding());
    entered.wait().await;
    fixture.login(1);
    let b = fixture.seed().await;
    release.release();
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    fixture.hub(1).set_offline(true);
    assert_eq!(
        fixture
            .vault()
            .get(b, request_binding())
            .await
            .unwrap()
            .expose(),
        "secret A"
    );
    assert!(matches!(
        fixture.vault().get(id, request_binding()).await,
        Err(VaultError::NotFound)
    ));
}

#[tokio::test]
async fn completed_vault_work_is_rechecked_before_its_caller_first_polls() {
    let fixture = NativeAccounts::new();
    let pending = fixture
        .vault()
        .put(None, "A".into(), request_binding(), Secret::new("A"));
    while fixture.count(0).await == 0 {
        tokio::task::yield_now().await;
    }
    // The native write/pull has no pending boundary, so it finishes on the same poll.
    tokio::task::yield_now().await;
    fixture.login(1);
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn an_unlock_response_cannot_install_keys_after_a_replacement_login() {
    let fixture = NativeAccounts::new();
    fixture.vault().lock();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::KeysetReady);
    let pending = fixture
        .vault()
        .unlock(Secret::new(crate::account::testing::PASSWORD));
    entered.wait().await;
    fixture.login(1);
    let current = fixture.source();
    release.release();
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    assert!(current.check_current().is_ok());
    assert_eq!(fixture.vault().status(), VaultStatus::Unlocked);
}

#[tokio::test]
async fn an_explicit_lock_cancels_an_unlock_already_waiting_for_its_record() {
    let fixture = NativeAccounts::new();
    fixture.vault().lock();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::KeysetReady);
    let pending = fixture
        .vault()
        .unlock(Secret::new(crate::account::testing::PASSWORD));
    entered.wait().await;
    fixture.vault().lock();
    release.release();
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    assert_eq!(fixture.vault().status(), VaultStatus::Locked);
    fixture
        .vault()
        .unlock(Secret::new(crate::account::testing::PASSWORD))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unlock_completed_before_caller_poll_never_publishes_success_for_b() {
    let fixture = NativeAccounts::new();
    fixture.vault().lock();
    let revision = fixture.vault().revision();
    let pending = fixture
        .vault()
        .unlock(Secret::new(crate::account::testing::PASSWORD));
    fixture.vault().changed(revision).await;
    assert_eq!(fixture.vault().status(), VaultStatus::Unlocked);
    fixture.login(1);
    assert!(matches!(pending.await, Err(VaultError::Stale)));
    assert!(fixture.source().check_current().is_ok());
}

#[tokio::test]
async fn same_account_relogin_and_a_b_a_never_revive_a_pending_vault_read() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    let old = fixture.vault().get(id, request_binding());
    fixture.login(0);
    assert!(matches!(old.await, Err(VaultError::Stale)));
    let old = fixture.vault().get(id, request_binding());
    fixture.login(1);
    fixture.login(0);
    assert!(matches!(old.await, Err(VaultError::Stale)));
    assert_eq!(
        fixture
            .vault()
            .get(id, request_binding())
            .await
            .unwrap()
            .expose(),
        "secret A"
    );
}

#[tokio::test(start_paused = true)]
async fn an_old_account_idle_timer_cannot_lock_a_new_account_without_activity() {
    let fixture = NativeAccounts::new();
    fixture.vault().set_auto_lock(Duration::from_secs(100));
    tokio::task::yield_now().await;
    fixture.login(1);
    tokio::time::advance(Duration::from_secs(101)).await;
    tokio::task::yield_now().await;
    assert_eq!(fixture.vault().status(), VaultStatus::Unlocked);
}

#[tokio::test(start_paused = true)]
async fn a_late_a_completion_cannot_extend_or_cancel_b_idle_deadline() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    fixture.vault().set_auto_lock(Duration::from_secs(100));
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::ChangesReady);
    let old = fixture.vault().get(id, request_binding());
    entered.wait().await;
    fixture.login(1);
    fixture.seed().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(40)).await;
    release.release();
    assert!(matches!(old.await, Err(VaultError::Stale)));
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(fixture.vault().status(), VaultStatus::Locked);
}

#[tokio::test(start_paused = true)]
async fn captured_terminal_credential_activity_refreshes_only_its_own_idle_timer() {
    let fixture = NativeAccounts::new();
    let id = fixture.seed().await;
    fixture.vault().set_auto_lock(Duration::from_secs(100));
    tokio::task::yield_now().await;
    let store = fixture.store();
    tokio::time::advance(Duration::from_secs(40)).await;
    store
        .get(
            id,
            &nocterm_session::SecretRequest::Password {
                target: Target::new("ann", "example.test", 22),
                retry: false,
            },
        )
        .await
        .unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(fixture.vault().status(), VaultStatus::Unlocked);
    tokio::time::advance(Duration::from_secs(40)).await;
    tokio::task::yield_now().await;
    assert_eq!(fixture.vault().status(), VaultStatus::Locked);
}

fn request_binding() -> CredentialBinding {
    CredentialBinding::Password {
        target: Target::new("ann", "example.test", 22),
    }
}

#[tokio::test]
async fn a_delayed_original_revocation_cannot_discard_a_replacement_or_its_idle_owner() {
    for replacements in [vec![0], vec![1], vec![1, 0]] {
        let fixture = NativeAccounts::new();
        let id = fixture.seed().await;
        let (entered, release) = fixture.hub(0).pause_next_error(
            StoreBoundary::ChangesReady,
            nocterm_hubs_model::HubError::Revoked,
        );
        let pending = fixture.vault().get(id, request_binding());
        entered.wait().await;
        for index in replacements {
            fixture.login(index);
        }
        let current = fixture.source();
        release.release();
        assert!(matches!(pending.await, Err(VaultError::Stale)));
        assert!(current.check_current().is_ok());
        assert_eq!(fixture.vault().status(), VaultStatus::Unlocked);
        fixture.seed().await;
        assert!(current.check_current().is_ok());
    }
}
