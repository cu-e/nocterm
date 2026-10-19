//! Shared concrete errors whose physical allocation retains its supplied credit.
//!
//! This primitive owns one private Arc containing the original error and credit.
//! Callers prepay `allocation_bytes()` before `new`; allocations already owned by
//! the error are separate liabilities. No raw Arc or Weak can escape this owner.
use std::{
    error::Error,
    fmt,
    mem::size_of,
    sync::{Arc, atomic::AtomicUsize},
};

struct Body<E, Credit> {
    error: E,
    // Declaration order also applies when the error's destructor unwinds.
    _credit: Credit,
}
// Pinned std ArcInner is repr(C), with atomic counts before aligned payload.
#[repr(C)]
struct ArcLayout<T> {
    _strong: AtomicUsize,
    _weak: AtomicUsize,
    _value: T,
}
pub(crate) const fn shared_allocation_bytes<T>() -> usize {
    size_of::<ArcLayout<T>>()
}

/// Cloneable immutable diagnosis with inseparable physical lifetime credit.
///
/// Clone preserves the original error's address without requiring either the
/// error or its credit to implement Clone. The last handle physically frees the
/// Arc block, drops the original error, and finally releases its supplied credit.
pub struct SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    inner: Option<Arc<Body<E, Credit>>>,
}
impl<E, Credit> SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    /// Exact requested bytes of this owner's private shared allocation.
    ///
    /// The caller reserves these bytes before construction. This includes the
    /// inline error and credit fields, but excludes their separately owned heap.
    /// Independent allocator checks must validate the pinned std layout.
    pub const fn allocation_bytes() -> usize {
        shared_allocation_bytes::<Body<E, Credit>>()
    }

    /// Construct after the caller has admitted the actual shared allocation.
    /// The credit must cover this block and any other liability it represents;
    /// supplying an arbitrary preexisting error does not prepay that error's heap.
    pub fn new(error: E, credit: Credit) -> Self {
        Self {
            inner: Some(Arc::new(Body {
                error,
                _credit: credit,
            })),
        }
    }
}
impl<E, Credit> Clone for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.as_ref().map(Arc::clone),
        }
    }
}
impl<E, Credit> AsRef<E> for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn as_ref(&self) -> &E {
        &self.inner.as_deref().expect("live resident error").error
    }
}
impl<E, Credit> fmt::Debug for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_ref(), f)
    }
}
impl<E, Credit> fmt::Display for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_ref(), f)
    }
}
impl<E, Credit> Error for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.as_ref())
    }
}
impl<E, Credit> Drop for SharedResidentError<E, Credit>
where
    E: Error + Send + Sync + 'static,
    Credit: Send + Sync + 'static,
{
    fn drop(&mut self) {
        // No Weak escapes, so the final into_inner deallocates the actual Arc
        // block before the returned Body drops error then credit. Into_inner,
        // unlike try_unwrap, guarantees one winner among concurrent last drops.
        if let Some(body) = self.inner.take().and_then(Arc::into_inner) {
            drop(body);
        }
    }
}

#[cfg(test)]
mod tests;
