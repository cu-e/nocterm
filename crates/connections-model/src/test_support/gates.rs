//! One reusable explicitly driven blocking gate for repository and observation ports.
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub struct BlockingGate {
    released: Mutex<bool>,
    changed: Condvar,
    entered: AtomicUsize,
    returned: AtomicUsize,
}
impl BlockingGate {
    pub fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }
    pub fn returned(&self) -> usize {
        self.returned.load(Ordering::SeqCst)
    }
    pub(crate) fn wait(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
    }
    pub(crate) fn complete(&self) {
        self.returned.fetch_add(1, Ordering::SeqCst);
    }
    pub fn release_on_drop(self: &Arc<Self>) -> BlockingRelease {
        BlockingRelease { gate: self.clone() }
    }
}
pub struct BlockingRelease {
    gate: Arc<BlockingGate>,
}
impl Drop for BlockingRelease {
    fn drop(&mut self) {
        self.gate.release();
    }
}
