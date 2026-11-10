//! Opaque allocation lifetime carriers shared by actual native quota owners.
//!
//! Counts describe the owner's real reservation. This module creates no quota,
//! authenticates no source, and grants no execution or publication authority.
use std::{fmt, sync::Arc};

struct RetainedAllocation {
    bytes: usize,
    payloads: usize,
    _keepalive: Box<dyn Send + Sync + 'static>,
}
/// Clones retain one actual reservation until the final lease is dropped.
#[derive(Clone)]
pub struct AllocationLease(Arc<RetainedAllocation>);
impl AllocationLease {
    /// The supplied owner must retain the actual permits matching these immutable
    /// counts. Descriptive counts or an arbitrary keepalive do not prove admission.
    pub fn new(bytes: usize, payloads: usize, keepalive: impl Send + Sync + 'static) -> Self {
        Self(Arc::new(RetainedAllocation {
            bytes,
            payloads,
            _keepalive: Box::new(keepalive),
        }))
    }
    pub fn bytes(&self) -> usize {
        self.0.bytes
    }
    pub fn payloads(&self) -> usize {
        self.0.payloads
    }
}
impl fmt::Debug for AllocationLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AllocationLease")
            .field("bytes", &self.bytes())
            .field("payloads", &self.payloads())
            .finish_non_exhaustive()
    }
}
/// Owns the actual value and its reservation together. Share this generation with
/// `Arc<Allocated<T>>`; a deep clone of T requires a separate real reservation.
/// There is deliberately no Clone or uncharged consuming extraction API.
pub struct Allocated<T> {
    // Field order keeps the reservation alive through destruction of the value.
    value: T,
    allocation: AllocationLease,
}
impl<T> Allocated<T> {
    pub fn new(value: T, allocation: AllocationLease) -> Self {
        Self { value, allocation }
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn allocation(&self) -> &AllocationLease {
        &self.allocation
    }
    /// Retains the same reservation throughout a consuming transformation.
    /// The owner must first reserve separately if the transformation can exceed
    /// this reservation or retain another independently allocated generation.
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> Allocated<U> {
        let Self { value, allocation } = self;
        Allocated {
            value: transform(value),
            allocation,
        }
    }
}
impl<T> fmt::Debug for Allocated<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Allocated")
            .field("allocation", &self.allocation)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn shared_generations_keep_one_actual_reservation_until_the_last_reference_drops() {
        let released = Arc::new(AtomicUsize::new(0));
        let allocation = AllocationLease::new(8, 1, Guard(released.clone()));
        let observer = allocation.clone();
        let value = Arc::new(Allocated::new(7u32, allocation));
        let native_worker = value.clone();
        drop(value);
        drop(observer);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        assert_eq!(
            (
                *native_worker.value(),
                native_worker.allocation().bytes(),
                native_worker.allocation().payloads()
            ),
            (7, 8, 1)
        );
        drop(native_worker);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn consuming_a_value_transfers_its_allocation_without_an_uncharged_gap() {
        let released = Arc::new(AtomicUsize::new(0));
        let value = Allocated::new(3u32, AllocationLease::new(8, 0, Guard(released.clone())));
        let changed = value.map(|value| {
            assert_eq!(released.load(Ordering::SeqCst), 0);
            u64::from(value) + 1
        });
        assert_eq!(*changed.value(), 4);
        assert_eq!(released.load(Ordering::SeqCst), 0);
        drop(changed);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
