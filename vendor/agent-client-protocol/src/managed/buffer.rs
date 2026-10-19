//! Known-layout, fallible replacement backing with physical drop ordering.
use super::{AllocationKind, Budget, ManagedError};
use std::alloc::Layout;

/// An exactly reserved vector whose backing dies before its native credit.
///
/// Values supplied by callers must themselves retain their physical ownership.
/// This owner pays the vector backing, never an estimate of a child's payload.
pub struct Buffer<T, B: Budget> {
    values: Vec<T>,
    credit: Option<B::Credit>,
    budget: B,
    kind: AllocationKind,
}
impl<T, B: Budget> Buffer<T, B> {
    pub(crate) fn belongs_to(&self, budget: &B) -> bool {
        self.budget.same_source(budget)
    }
    /// Create empty inline storage without requesting a heap allocation.
    pub fn new(budget: B) -> Self {
        Self::with_kind(budget, AllocationKind::Collection)
    }
    pub(crate) fn with_kind(budget: B, kind: AllocationKind) -> Self {
        Self {
            values: Vec::new(),
            credit: None,
            budget,
            kind,
        }
    }
    /// Reserve exact replacement capacity while retaining the previous backing.
    pub fn reserve(&mut self, additional: usize) -> Result<(), ManagedError<B::Error>> {
        let needed = self
            .values
            .len()
            .checked_add(additional)
            .ok_or(ManagedError::CapacityOverflow)?;
        if needed <= self.values.capacity() {
            return Ok(());
        }
        let layout = Layout::array::<T>(needed).map_err(|_| ManagedError::CapacityOverflow)?;
        let credit = self
            .budget
            .reserve(layout, self.kind)
            .map_err(ManagedError::Refused)?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(needed)
            .map_err(|_| ManagedError::AllocationFailed)?;
        replacement.append(&mut self.values);
        let previous = std::mem::replace(&mut self.values, replacement);
        drop(previous);
        self.credit = Some(credit);
        Ok(())
    }
    /// Grow incrementally with amortized replacement work. If spare capacity
    /// alone is refused, retry only the exact required capacity; headroom is
    /// never a new native admission floor.
    pub fn reserve_amortized(&mut self, additional: usize) -> Result<(), ManagedError<B::Error>> {
        let needed = self
            .len()
            .checked_add(additional)
            .ok_or(ManagedError::CapacityOverflow)?;
        if needed <= self.capacity() {
            return Ok(());
        }
        let capacity = self
            .capacity()
            .checked_mul(2)
            .unwrap_or(needed)
            .max(needed)
            .max(1);
        match self.reserve(capacity - self.len()) {
            Err(ManagedError::Refused(_)) if capacity > needed => self.reserve(additional),
            result => result,
        }
    }
    /// Append only after enough replacement capacity has been approved.
    pub fn push(&mut self, value: T) -> Result<(), (ManagedError<B::Error>, T)> {
        if let Err(error) = self.reserve_amortized(1) {
            return Err((error, value));
        }
        self.values.push(value);
        Ok(())
    }
    /// Remove and return the final value without allocating.
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    /// Remove the given original slot, retaining the order of all survivors.
    pub fn remove(&mut self, index: usize) -> T {
        self.values.remove(index)
    }
    /// Borrow actual retained values.
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    /// Borrow actual retained values for an in-place scalar transition.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }
    /// Number of actual retained values.
    pub fn len(&self) -> usize {
        self.values.len()
    }
    /// Whether no values remain.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// Actual requested backing capacity.
    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }
    /// Destroy all values while retaining the paid backing for reuse.
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub(crate) fn into_parts(self) -> (Vec<T>, Option<B::Credit>, B) {
        (self.values, self.credit, self.budget)
    }
}

/// An owning iterator retaining its original vector backing credit through Drop.
/// Dropping it early destroys all remaining values and frees backing first.
pub struct Values<T, B: Budget> {
    values: std::vec::IntoIter<T>,
    _credit: Option<B::Credit>,
    _budget: B,
}
impl<T, B: Budget> Iterator for Values<T, B> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}
impl<T, B: Budget> Buffer<T, B> {
    /// Move actual values into an iterator without copying or allocating.
    pub fn into_values(self) -> Values<T, B> {
        let (values, credit, budget) = self.into_parts();
        Values {
            values: values.into_iter(),
            _credit: credit,
            _budget: budget,
        }
    }
}
