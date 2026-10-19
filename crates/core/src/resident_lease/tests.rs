use super::*;
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

struct Credit(Arc<AtomicUsize>);
impl Drop for Credit {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn payment_survives_clones_and_returns_once_after_the_last_handle() {
    let released = Arc::new(AtomicUsize::new(0));
    let original = SharedResidentLease::new(Credit(released.clone()));
    let last = original.clone();
    drop(original);
    assert_eq!(released.load(Ordering::SeqCst), 0);
    drop(last);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_last_handles_return_the_original_payment_once() {
    let released = Arc::new(AtomicUsize::new(0));
    let original = SharedResidentLease::new(Credit(released.clone()));
    let other = original.clone();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = barrier.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        drop(other);
    });
    barrier.wait();
    drop(original);
    worker.join().unwrap();
    assert_eq!(released.load(Ordering::SeqCst), 1);
}
