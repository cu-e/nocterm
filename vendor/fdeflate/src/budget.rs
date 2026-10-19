//! Explicit, instance-local admission for native decoder allocations.
//!
//! Implementations admit bytes before allocation and retire them only after the
//! corresponding physical backing has been destroyed. No global state is used.
use std::{fmt, mem};

/// A refused allocation. The caller may retain a more specific original cause.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationRefused;
impl fmt::Display for AllocationRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("native decoder allocation refused")
    }
}
impl std::error::Error for AllocationRefused {}

/// An explicit allocation account borrowed or owned by one decoder instance.
///
/// Clone must refer to the same account. A refusal must remain observable by the
/// operation driver even if a format parser ignores an optional metadata error.
pub trait AllocationBudget: Clone {
    /// Admit a new physical allocation before it is made.
    fn admit(&self, bytes: usize) -> Result<(), AllocationRefused>;
    /// Retire an admitted allocation after its physical backing has been freed.
    fn retire(&self, bytes: usize);
    /// Whether allocations use explicitly admitted replacement backings.
    fn tracked(&self) -> bool {
        true
    }
}

/// Original decoder behavior, without an external admission account.
#[derive(Clone, Copy, Debug, Default)]
pub struct Untracked;
impl AllocationBudget for Untracked {
    fn admit(&self, _: usize) -> Result<(), AllocationRefused> {
        Ok(())
    }
    fn retire(&self, _: usize) {}
    fn tracked(&self) -> bool {
        false
    }
}

/// An inline receipt. Store this AFTER its physical backing in an owning type.
///
/// It allocates no header and must never be detached from a live backing.
pub struct Allocation<B: AllocationBudget> {
    budget: B,
    bytes: usize,
}
impl<B: AllocationBudget> Allocation<B> {
    /// Admit before constructing the corresponding backing.
    pub fn new(budget: B, bytes: usize) -> Result<Self, AllocationRefused> {
        if bytes != 0 {
            budget.admit(bytes)?;
        }
        Ok(Self { budget, bytes })
    }
    /// Merge an already admitted receipt after its backing has moved into this owner.
    pub fn absorb(&mut self, mut other: Self) {
        self.bytes = self
            .bytes
            .checked_add(other.bytes)
            .expect("admitted allocation sum overflow");
        other.bytes = 0;
    }
    /// Borrow the original account for an independently admitted child owner.
    pub fn budget(&self) -> &B {
        &self.budget
    }
    /// Grow a vector using an independently admitted replacement allocation.
    /// The original backing is physically freed before its receipt is retired.
    pub fn reserve<T>(
        &mut self,
        value: &mut Vec<T>,
        required: usize,
    ) -> Result<(), AllocationRefused> {
        if !self.budget.tracked() || required <= value.capacity() {
            return Ok(());
        }
        let bytes = required
            .checked_mul(mem::size_of::<T>())
            .ok_or(AllocationRefused)?;
        let next = Self::new(self.budget.clone(), bytes)?;
        let mut replacement = Vec::with_capacity(required);
        replacement.append(value);
        let old = mem::replace(value, replacement);
        drop(old);
        let old = mem::replace(self, next);
        drop(old);
        Ok(())
    }
}
impl<B: AllocationBudget> Drop for Allocation<B> {
    fn drop(&mut self) {
        if self.bytes != 0 {
            self.budget.retire(self.bytes);
        }
    }
}
impl<B: AllocationBudget> fmt::Debug for Allocation<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Allocation")
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// A vector and its inseparable inline allocation receipt.
///
/// Native format owners transferring this into another owner must keep the
/// returned receipt AFTER the physical vector and destroy both together.
pub struct BudgetVec<T, B: AllocationBudget> {
    value: Vec<T>,
    allocation: Allocation<B>,
}
impl<T, B: AllocationBudget> BudgetVec<T, B> {
    /// Create an empty owned vector without allocating.
    pub fn new(budget: B) -> Result<Self, AllocationRefused> {
        Ok(Self {
            value: Vec::new(),
            allocation: Allocation::new(budget, 0)?,
        })
    }
    /// Admit an initial vector capacity before constructing its physical backing.
    pub fn with_capacity(budget: B, capacity: usize) -> Result<Self, AllocationRefused> {
        let bytes = capacity
            .checked_mul(mem::size_of::<T>())
            .ok_or(AllocationRefused)?;
        let allocation = Allocation::new(budget, bytes)?;
        let value = Vec::with_capacity(capacity);
        Ok(Self { value, allocation })
    }
    /// Take this vector, leaving an empty owner sharing the same original account.
    pub fn take(&mut self) -> Self {
        let empty =
            Self::new(self.allocation.budget().clone()).expect("empty owner allocates nothing");
        mem::replace(self, empty)
    }
    /// Discard initialized elements while retaining the paid physical capacity.
    pub fn clear(&mut self) {
        self.value.clear();
    }
    /// Borrow initialized elements.
    pub fn as_slice(&self) -> &[T] {
        &self.value
    }
    /// Borrow initialized elements mutably without exposing capacity-changing operations.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.value
    }
    /// Grow before initializing new elements; shrinking keeps its real capacity.
    pub fn resize(&mut self, len: usize, value: T) -> Result<(), AllocationRefused>
    where
        T: Clone,
    {
        self.allocation.reserve(&mut self.value, len)?;
        self.value.resize(len, value);
        Ok(())
    }
    /// Move physical backing and receipt to another native format owner.
    pub fn into_parts(self) -> (Vec<T>, Allocation<B>) {
        (self.value, self.allocation)
    }
}
impl<T> BudgetVec<T, Untracked> {
    /// Return an ordinary vector only for the original untracked API.
    pub fn into_untracked(self) -> Vec<T> {
        self.value
    }
}
impl<T, B: AllocationBudget> std::ops::Deref for BudgetVec<T, B> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.value
    }
}
impl<T, B: AllocationBudget> std::ops::DerefMut for BudgetVec<T, B> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.value
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
