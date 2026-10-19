//! Shared payment retires only after its actual shared block is deallocated.
use std::sync::Arc;

/// A shared lifetime credit without a payload, mutable access or weak escape.
/// Callers admit this owner's header and represented physical backing first.
pub struct SharedResidentLease<Credit> {
    inner: Option<Arc<Credit>>,
}
impl<Credit> SharedResidentLease<Credit> {
    pub const fn allocation_bytes() -> usize {
        crate::resident_error::shared_allocation_bytes::<Credit>()
    }

    pub fn new(credit: Credit) -> Self {
        Self {
            inner: Some(Arc::new(credit)),
        }
    }
}
impl<Credit> Clone for SharedResidentLease<Credit> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.as_ref().map(Arc::clone),
        }
    }
}
impl<Credit> Drop for SharedResidentLease<Credit> {
    fn drop(&mut self) {
        if let Some(credit) = self.inner.take().and_then(Arc::into_inner) {
            drop(credit);
        }
    }
}

#[cfg(test)]
mod tests;
