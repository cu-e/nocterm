//! Outstanding work keeps its original counter even across release and reconnect.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub(super) struct OperationCounter {
    active: Arc<AtomicUsize>,
}
pub(crate) struct OperationGuard(Arc<AtomicUsize>);
impl OperationCounter {
    pub(super) fn hold(&self) -> OperationGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        OperationGuard(self.active.clone())
    }
    pub(super) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire) != 0
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
