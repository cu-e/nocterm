use super::*;
use futures::executor::block_on;
use nocterm_session::Target;
use std::time::Duration;

const MASTER: &str = "test unique master passphrase";

fn store(directory: &tempfile::TempDir) -> VaultCredentials {
    let service = Arc::new(
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
    );
    block_on(service.create(Secret::new(MASTER))).unwrap();
    VaultCredentials::new(service)
}

fn password(target: &Target, retry: bool) -> SecretRequest {
    SecretRequest::Password {
        target: target.clone(),
        retry,
    }
}

#[test]
fn a_saved_answer_returns_for_the_same_question_only() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(&directory);
    let target = Target::new("test", "test-host", 22);
    let id = block_on(store.save(
        "test".into(),
        &password(&target, false),
        Secret::new("stored account password"),
    ))
    .unwrap();
    let secret = block_on(store.get(id, &password(&target, true))).unwrap();
    assert_eq!(secret.expose(), "stored account password");
    let other = password(&Target::new("other", "other-host", 22), false);
    let error = block_on(store.get(id, &other)).unwrap_err();
    assert!(error.contains("does not match"), "{error}");
}

#[test]
fn one_time_answers_are_never_saved() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(&directory);
    let mfa = SecretRequest::Interactive {
        prompt: "MFA".into(),
        echo: false,
    };
    assert!(block_on(store.save("test".into(), &mfa, Secret::new("123456"))).is_err());
    assert!(block_on(store.0.list()).unwrap().is_empty());
}

#[test]
fn the_store_becomes_available_when_the_vault_unlocks() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(&directory);
    store.0.lock();
    assert!(!store.is_available());
    let waiting = store.wait_available();
    block_on(store.0.unlock(Secret::new(MASTER))).unwrap();
    block_on(waiting);
    assert!(store.is_available());
}
