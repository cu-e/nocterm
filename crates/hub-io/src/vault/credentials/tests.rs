use super::*;
use crate::account::testing::{self, NativeAccounts as Fixture};
use crate::store::memory::StoreBoundary;
use nocterm_hubs_model::source_identity::{LocalAccountPartition, LocalAccountSourceError};
use nocterm_session::Target;

fn request() -> SecretRequest {
    SecretRequest::Password {
        target: Target::new("ann", "example.test", 22),
        retry: false,
    }
}

#[tokio::test]
async fn a_captured_store_reads_and_saves_only_its_account() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let id = fixture.seed().await;
    assert_eq!(
        store.get(id, &request()).await.unwrap().expose(),
        "secret A"
    );
    fixture
        .subjects()
        .renew(fixture.slot().hub().clone(), testing::ACCOUNT_ID, "device");
    assert!(store.is_available());
    assert_eq!(
        store.get(id, &request()).await.unwrap().expose(),
        "secret A"
    );
    assert_eq!(fixture.count(0).await, 1);
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn a_worker_queued_before_an_account_change_never_looks_up_b() {
    let fixture = Fixture::new();
    let id = fixture.seed().await;
    let store = fixture.store();
    let pending = store.get(id, &request());
    fixture.login(1);
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
    assert_eq!(fixture.count(1).await, 0);
    assert!(!store.is_available());
}

#[tokio::test]
async fn a_queued_save_does_not_follow_a_new_login() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let pending = store.save("late".into(), &request(), Secret::new("secret A"));
    fixture.login(1);
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
    assert_eq!(fixture.count(0).await, 0);
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn a_late_read_cannot_publish_a_cache_into_b() {
    let fixture = Fixture::new();
    let id = fixture.seed().await;
    let store = fixture.store();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::ChangesReady);
    let pending = store.get(id, &request());
    entered.wait().await;
    fixture.login(1);
    let b_id = fixture
        .store()
        .save("B".into(), &request(), Secret::new("secret B"))
        .await
        .unwrap();
    release.release();
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
    assert_eq!(
        fixture
            .store()
            .get(b_id, &request())
            .await
            .unwrap()
            .expose(),
        "secret B"
    );
    fixture.hub(1).set_offline(true);
    assert_eq!(
        fixture
            .store()
            .get(b_id, &request())
            .await
            .unwrap()
            .expose(),
        "secret B"
    );
}

#[tokio::test]
async fn an_admitted_remote_save_stays_on_a_and_cannot_replace_b_cache() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::Put);
    let pending = store.save("late A".into(), &request(), Secret::new("secret A"));
    entered.wait().await;
    fixture.login(1);
    let b_id = fixture
        .store()
        .save("B".into(), &request(), Secret::new("secret B"))
        .await
        .unwrap();
    release.release();
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
    assert_eq!(fixture.count(0).await, 1);
    assert_eq!(fixture.count(1).await, 1);
    fixture.hub(1).set_offline(true);
    assert_eq!(
        fixture
            .store()
            .get(b_id, &request())
            .await
            .unwrap()
            .expose(),
        "secret B"
    );
}

#[tokio::test]
async fn lock_drops_keys_even_while_a_captured_store_is_retained() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let keys = Arc::downgrade(&fixture.slot().open().unwrap());
    fixture.slot().lock();
    assert!(keys.upgrade().is_none());
    assert!(!store.is_available());
    assert!(matches!(
        store.save("A".into(), &request(), Secret::new("A")).await,
        Err(CredentialStoreError::Stale)
    ));
    fixture.login(0);
    assert!(!store.is_available());
    assert!(fixture.store().is_available());
}

#[tokio::test]
async fn a_to_b_to_a_and_eviction_never_revive_an_old_store() {
    let fixture = Fixture::new();
    let store = fixture.store();
    fixture.login(1);
    fixture.login(0);
    assert!(!store.is_available());
    assert!(matches!(
        store
            .get(CredentialId::from_bytes([1; 16]), &request())
            .await,
        Err(CredentialStoreError::Stale)
    ));
    let current = fixture.store();
    fixture.subjects().evict(fixture.slot().hub());
    assert!(!current.is_available());
    assert!(matches!(
        current.save("A".into(), &request(), Secret::new("A")).await,
        Err(CredentialStoreError::Stale)
    ));
}

#[tokio::test]
async fn same_account_new_login_invalidates_before_replacing_keys() {
    let fixture = Fixture::new();
    let store = fixture.store();
    fixture
        .subjects()
        .sign_in(fixture.slot().hub().clone(), testing::ACCOUNT_ID, "device");
    assert!(!store.is_available());
    assert!(matches!(
        store.save("A".into(), &request(), Secret::new("A")).await,
        Err(CredentialStoreError::Stale)
    ));
    assert_eq!(fixture.count(0).await, 0);
}

#[tokio::test]
async fn a_completed_worker_result_is_rechecked_when_the_future_is_polled() {
    let fixture = Fixture::new();
    let id = fixture.seed().await;
    let store = fixture.store();
    let keys = Arc::downgrade(&fixture.slot().open().unwrap());
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::Collections);
    let pending = store.get(id, &request());
    entered.wait().await;
    assert!(
        keys.strong_count() > 1,
        "the entered worker holds its original key lease"
    );
    release.release();
    // On this single-thread executor the final key lease drops in the same
    // poll that sends the worker result. Wait for that observable release,
    // rather than assuming a number of scheduler yields completed the worker.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while keys.strong_count() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the worker released its key lease before result polling");
    fixture.login(1);
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
}

struct RestrictedSource {
    source: Arc<dyn LocalAccountSource>,
    allowed: std::sync::atomic::AtomicBool,
}
impl RestrictedSource {
    fn deny(&self) {
        self.allowed
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
impl LocalAccountSource for RestrictedSource {
    fn partition(&self) -> LocalAccountPartition {
        self.source.partition()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        self.source.check_current()?;
        if self.allowed.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(())
        } else {
            Err(LocalAccountSourceError::Stale)
        }
    }
}

struct DescribedSource(LocalAccountPartition);
impl LocalAccountSource for DescribedSource {
    fn partition(&self) -> LocalAccountPartition {
        self.0.clone()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        Ok(())
    }
}

#[tokio::test]
async fn a_caller_partition_cannot_select_credentials_of_another_native_account() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let described = Arc::new(DescribedSource(source.partition()));
    fixture.login(1);
    assert!(matches!(
        fixture.capture_credentials(source),
        Err(HubError::NotSignedIn)
    ));
    assert!(matches!(
        fixture.capture_credentials(described),
        Err(HubError::Stale)
    ));
    assert!(fixture.store().is_available());
}

#[tokio::test]
async fn an_app_availability_wrapper_can_only_restrict_native_credentials() {
    let fixture = Fixture::new();
    let restriction = Arc::new(RestrictedSource {
        source: fixture.source(),
        allowed: true.into(),
    });
    let store = fixture.capture_credentials(restriction.clone()).unwrap();
    let id = fixture.seed().await;
    assert_eq!(
        store.get(id, &request()).await.unwrap().expose(),
        "secret A"
    );
    restriction.deny();
    assert!(!store.is_available());
    assert!(matches!(
        store
            .save("denied".into(), &request(), Secret::new("secret"))
            .await,
        Err(CredentialStoreError::Stale)
    ));
    assert_eq!(fixture.count(0).await, 1);
}

#[tokio::test]
async fn in_flight_keys_are_dropped_after_a_locked_operation_finishes() {
    let fixture = Fixture::new();
    let id = fixture.seed().await;
    let store = fixture.store();
    let keys = Arc::downgrade(&fixture.slot().open().unwrap());
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::ChangesReady);
    let pending = store.get(id, &request());
    entered.wait().await;
    fixture.slot().lock();
    assert!(
        keys.upgrade().is_some(),
        "only the in-flight worker holds the original keys"
    );
    release.release();
    assert!(matches!(pending.await, Err(CredentialStoreError::Stale)));
    assert!(keys.upgrade().is_none());
    assert!(!store.is_available());
}

#[tokio::test]
async fn interactive_answers_and_private_keys_are_never_saved_from_a_prompt() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let requests = [
        SecretRequest::Interactive {
            prompt: "MFA".into(),
            echo: false,
        },
        SecretRequest::PrivateKey {
            credential: CredentialId::from_bytes([1; 16]),
            public_key: "ssh-ed25519 exact".into(),
        },
    ];
    for request in requests {
        assert!(matches!(
            store
                .save("denied".into(), &request, Secret::new("secret"))
                .await,
            Err(CredentialStoreError::NotStorable)
        ));
    }
    assert_eq!(fixture.count(0).await, 0);
    assert_eq!(fixture.count(1).await, 0);
}

#[tokio::test]
async fn a_captured_revocation_cannot_erase_replacement_keys_or_cache() {
    let fixture = Fixture::new();
    let original = fixture.slot().open().unwrap();
    fixture.login(1);
    let store = fixture.store();
    let id = store
        .save("B".into(), &request(), Secret::new("secret B"))
        .await
        .unwrap();
    fixture.slot().erase_if(&original).unwrap();
    fixture.hub(1).set_offline(true);
    assert!(store.is_available());
    assert_eq!(
        store.get(id, &request()).await.unwrap().expose(),
        "secret B"
    );
    let current = fixture.slot().open().unwrap();
    fixture.slot().erase_if(&current).unwrap();
    assert!(!store.is_available());
    assert_eq!(fixture.slot().status(), nocterm_vault::VaultStatus::Missing);
}
