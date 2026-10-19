use crate::testing::{FakeCredentialError, FakeCredentialStore, Gate};
use crate::{CredentialStore, Secret, SecretRequest, Target};
use futures::{FutureExt as _, executor::block_on};
fn password(host: &str, retry: bool) -> SecretRequest {
    SecretRequest::Password {
        target: Target {
            host: host.into(),
            port: 22,
            user: "me".into(),
        },
        retry,
    }
}

#[test]
fn bindings_match_identity_ignore_retry_and_never_save_mfa() {
    let store = FakeCredentialStore::default();
    let id = store
        .insert(
            "server".into(),
            &password("host", false),
            Secret::new("secret"),
        )
        .unwrap();
    assert_eq!(
        store.lookup(id, &password("host", true)).unwrap(),
        Secret::new("secret")
    );
    assert_eq!(
        store.lookup(id, &password("another", false)),
        Err(FakeCredentialError::BindingMismatch)
    );
    let mfa = SecretRequest::Interactive {
        prompt: "Code".into(),
        echo: false,
    };
    assert_eq!(
        store.insert("mfa".into(), &mfa, Secret::new("123")),
        Err(FakeCredentialError::NotStorable)
    );
    assert_eq!(store.labels(), ["server"]);
    assert!(
        !format!("{:?}", store.lookup(id, &password("host", false)).unwrap()).contains("secret")
    );
}

#[test]
fn availability_broadcasts_to_all_waiters_without_polling_and_relocking_blocks_again() {
    let store = FakeCredentialStore::default();
    store.set_available(false);
    assert!(!store.is_available());
    let mut first = store.wait_available();
    let mut second = store.wait_available();
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    store.set_available(true);
    store.set_available(false);
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    store.set_available(true);
    block_on(async {
        futures::join!(first, second);
    });
    store.set_available(false);
    assert_eq!(
        store.insert("locked".into(), &password("host", false), Secret::new("s")),
        Err(FakeCredentialError::Locked)
    );
}

#[test]
fn suspended_store_operation_checks_availability_after_release_and_failure_is_one_shot() {
    let store = FakeCredentialStore::default();
    let gate = Gate::pending();
    store.suspend(gate.clone());
    let mut saving = store.save("server".into(), &password("host", false), Secret::new("s"));
    assert!(saving.as_mut().now_or_never().is_none());
    store.set_available(false);
    gate.release();
    assert!(block_on(saving).is_err());
    store.set_available(true);
    store.fail_next(FakeCredentialError::Exhausted);
    assert_eq!(
        store.insert("server".into(), &password("host", false), Secret::new("s")),
        Err(FakeCredentialError::Exhausted)
    );
    let id =
        block_on(store.save("server".into(), &password("host", false), Secret::new("s"))).unwrap();
    assert_eq!(
        block_on(store.get(id, &password("host", true))).unwrap(),
        Secret::new("s")
    );
}
