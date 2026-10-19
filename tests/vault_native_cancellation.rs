use nocterm_vault_store::VaultService;
use std::{sync::mpsc, time::Duration};
#[test]
fn cancel_releases_single_vault_worker_for_password_without_native_resume() {
    let directory = tempfile::tempdir().unwrap();
    let (provider, probe) = nocterm_device_unlock::testing::scoped_cancellation_provider();
    let service = VaultService::new_with_device_unlock(
        directory.path().join("vault.bin"),
        Duration::from_secs(60),
        Some(provider.clone()),
    )
    .unwrap();
    futures::executor::block_on(
        service.create(nocterm_session::Secret::new("native test password")),
    )
    .unwrap();
    futures::executor::block_on(service.enable_device_unlock()).unwrap();
    service.lock();
    let native = service.unlock_with_device();
    probe.wait_started(Duration::from_secs(2)).unwrap();
    service.lock();
    assert!(!service.is_unlocked());
    let password = service.unlock(nocterm_session::Secret::new("native test password"));
    let (finished, completion) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let native = futures::executor::block_on(native);
        let password = futures::executor::block_on(password);
        finished.send((native, password)).unwrap();
    });
    let (native, password) = completion.recv_timeout(Duration::from_secs(2)).unwrap();
    waiter.join().unwrap();
    assert!(matches!(native, Err(nocterm_vault::VaultError::Cancelled)));
    password.unwrap();
    assert!(probe.was_invalidated());
    assert!(service.is_unlocked());
}
