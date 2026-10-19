//! Authentic shared allocation whose reservation survives its last weak header.
use super::{AllocationKind, Budget, ManagedError};
use std::{
    alloc::{Layout, alloc, dealloc},
    marker::PhantomData,
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::atomic::{AtomicUsize, Ordering, fence},
};

struct Allocation<T, C> {
    strong: AtomicUsize,
    weak: AtomicUsize,
    value: ManuallyDrop<T>,
    credit: ManuallyDrop<C>,
}
/// A genuine SDK-owned allocation, with a credit for its exact layout.
///
/// The value dies at the final strong release. The header's credit dies only
/// after the allocation itself is freed, including when weak handles survive.
pub struct Shared<T, B: Budget> {
    pointer: NonNull<Allocation<T, B::Credit>>,
    marker: PhantomData<Allocation<T, B::Credit>>,
}
/// A weak reference to an original managed allocation; it owns no payload.
pub struct Weak<T, B: Budget> {
    pointer: NonNull<Allocation<T, B::Credit>>,
    marker: PhantomData<Allocation<T, B::Credit>>,
}
// SAFETY: payloads are immutable through Shared; atomic counters synchronize
// destruction. The credit may be destroyed by whichever thread drops last.
unsafe impl<T: Send + Sync, B: Budget> Send for Shared<T, B> where B::Credit: Send + Sync {}
unsafe impl<T: Send + Sync, B: Budget> Sync for Shared<T, B> where B::Credit: Send + Sync {}
unsafe impl<T: Send + Sync, B: Budget> Send for Weak<T, B> where B::Credit: Send + Sync {}
unsafe impl<T: Send + Sync, B: Budget> Sync for Weak<T, B> where B::Credit: Send + Sync {}

impl<T, B: Budget> Shared<T, B> {
    /// Allocate the actual header only after the supplied authority approves it.
    pub fn new(value: T, budget: &B) -> Result<Self, ManagedError<B::Error>> {
        let layout = Layout::new::<Allocation<T, B::Credit>>();
        let credit = budget
            .reserve(layout, AllocationKind::Shared)
            .map_err(ManagedError::Refused)?;
        // SAFETY: layout is nonzero (two atomics), valid for Allocation.
        let pointer = NonNull::new(unsafe { alloc(layout) }.cast::<Allocation<T, B::Credit>>())
            .ok_or(ManagedError::AllocationFailed)?;
        // SAFETY: allocation was approved and points to fresh properly aligned storage.
        unsafe {
            pointer.as_ptr().write(Allocation {
                strong: AtomicUsize::new(1),
                weak: AtomicUsize::new(1),
                value: ManuallyDrop::new(value),
                credit: ManuallyDrop::new(credit),
            })
        };
        Ok(Self {
            pointer,
            marker: PhantomData,
        })
    }
    /// Borrow the original immutable payload.
    pub fn get(&self) -> &T {
        // SAFETY: this strong reference keeps the initialized payload alive.
        unsafe { &self.pointer.as_ref().value }
    }
    /// Retain the authentic allocation header without retaining its payload.
    pub fn downgrade(&self) -> Weak<T, B> {
        // SAFETY: this strong reference keeps the header alive.
        increment(unsafe { &self.pointer.as_ref().weak });
        Weak {
            pointer: self.pointer,
            marker: PhantomData,
        }
    }
    /// Compare genuine allocation identity, without inspecting source bytes.
    pub fn same_allocation(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
}
impl<T, B: Budget> Clone for Shared<T, B> {
    fn clone(&self) -> Self {
        // SAFETY: self holds a strong reference to the live header.
        increment(unsafe { &self.pointer.as_ref().strong });
        Self {
            pointer: self.pointer,
            marker: PhantomData,
        }
    }
}
impl<T, B: Budget> Clone for Weak<T, B> {
    fn clone(&self) -> Self {
        // SAFETY: self holds a weak reference to the live header.
        increment(unsafe { &self.pointer.as_ref().weak });
        Self {
            pointer: self.pointer,
            marker: PhantomData,
        }
    }
}
impl<T, B: Budget> Weak<T, B> {
    pub(crate) fn same_allocation(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    /// Upgrade only while an original strong owner is still alive.
    pub fn upgrade(&self) -> Option<Shared<T, B>> {
        // SAFETY: the weak handle pins the atomic header.
        let strong = unsafe { &self.pointer.as_ref().strong };
        let mut observed = strong.load(Ordering::Relaxed);
        loop {
            if observed == 0 {
                return None;
            }
            if observed > isize::MAX as usize {
                std::process::abort();
            }
            match strong.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Shared {
                        pointer: self.pointer,
                        marker: PhantomData,
                    });
                }
                Err(current) => observed = current,
            }
        }
    }
}
fn increment(counter: &AtomicUsize) {
    // Match the standard Arc overflow defense; this is not a source/grammar cap.
    if counter.fetch_add(1, Ordering::Relaxed) > isize::MAX as usize {
        std::process::abort();
    }
}
impl<T, B: Budget> Drop for Shared<T, B> {
    fn drop(&mut self) {
        // SAFETY: this reference pins the allocation until the implicit weak is released.
        let allocation = unsafe { self.pointer.as_ref() };
        if allocation.strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        // This guard releases the implicit weak even if the payload destructor panics.
        let implicit = Weak::<T, B> {
            pointer: self.pointer,
            marker: PhantomData,
        };
        // SAFETY: the last strong release uniquely owns payload destruction.
        unsafe { ManuallyDrop::drop(&mut (*self.pointer.as_ptr()).value) };
        drop(implicit);
    }
}
impl<T, B: Budget> Drop for Weak<T, B> {
    fn drop(&mut self) {
        // SAFETY: this weak handle pins the atomic header.
        if unsafe { self.pointer.as_ref().weak.fetch_sub(1, Ordering::Release) } != 1 {
            return;
        }
        fence(Ordering::Acquire);
        // SAFETY: final weak release uniquely owns header destruction. Move the
        // credit out, free the physical block, and only then refund its layout.
        unsafe {
            let credit = ptr::read(&*(*self.pointer.as_ptr()).credit);
            dealloc(
                self.pointer.as_ptr().cast(),
                Layout::new::<Allocation<T, B::Credit>>(),
            );
            drop(credit);
        }
    }
}
