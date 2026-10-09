use super::*;
use futures::{
    executor::block_on,
    task::{ArcWake, waker},
};
use std::task::{Context, Poll};

const MASTER: &str = "test unique master passphrase";

#[derive(Default)]
struct Woken(AtomicBool);
impl ArcWake for Woken {
    fn wake_by_ref(this: &Arc<Self>) {
        this.0.store(true, Ordering::SeqCst);
    }
}

fn locked_vault(directory: &tempfile::TempDir) -> VaultService {
    let service =
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap();
    block_on(service.create(Secret::new(MASTER))).unwrap();
    service.lock();
    service
}

#[test]
fn unlocking_on_the_worker_wakes_a_waiter_without_a_timer() {
    let directory = tempfile::tempdir().unwrap();
    let service = locked_vault(&directory);
    assert_eq!(service.status(), VaultStatus::Locked);
    let woken = Arc::new(Woken::default());
    let waker = waker(woken.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiting = service.wait_unlocked();
    assert!(waiting.as_mut().poll(&mut cx).is_pending());

    block_on(service.unlock(Secret::new(MASTER))).unwrap();
    assert!(woken.0.load(Ordering::SeqCst), "unlocking wakes the waiter");
    assert_eq!(waiting.as_mut().poll(&mut cx), Poll::Ready(()));
    assert_eq!(service.status(), VaultStatus::Unlocked);
}

#[test]
fn locking_publishes_a_change() {
    let directory = tempfile::tempdir().unwrap();
    let service = locked_vault(&directory);
    block_on(service.unlock(Secret::new(MASTER))).unwrap();
    let seen = service.revision();
    let woken = Arc::new(Woken::default());
    let waker = waker(woken.clone());
    let mut cx = Context::from_waker(&waker);
    let mut changed = service.changed(seen);
    assert!(changed.as_mut().poll(&mut cx).is_pending());

    service.lock();
    assert!(woken.0.load(Ordering::SeqCst));
    assert!(matches!(changed.as_mut().poll(&mut cx), Poll::Ready(now) if now != seen));
    assert_eq!(service.status(), VaultStatus::Locked);
}

#[test]
fn a_missing_vault_reports_missing() {
    let directory = tempfile::tempdir().unwrap();
    let service =
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap();
    assert_eq!(service.status(), VaultStatus::Missing);
}
