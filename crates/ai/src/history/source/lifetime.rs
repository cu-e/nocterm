//! The original Source loan encloses every physical native future owner.
use super::{ManagedFuture, SourceContext, SourceError, SourceLease};
use crate::{
    history::dispatch::ActiveDispatch,
    model::{
        json::{deferred::NativeAllocationError, lifetime::Shared},
        retention::document::{AllocationPlan, PendingAllocation},
    },
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

struct SourceLeaseBody {
    _header: PendingAllocation,
    context: SourceContext,
    _original: SourceLease,
}
/// Cloning aliases the single original Source loan, without allocating.
#[derive(Clone)]
pub struct SourceLifetime(Shared<SourceLeaseBody>);
impl SourceLifetime {
    pub(super) fn new(original: SourceLease) -> Result<Self, SourceError> {
        let context = original.context()?;
        let header = original
            .producer
            .0
            .scope
            .reserve_phase_metadata(AllocationPlan::shared::<SourceLeaseBody>())?;
        Ok(Self(Shared::new(SourceLeaseBody {
            _header: header,
            context,
            _original: original,
        })))
    }
    pub fn context_ref(&self) -> &SourceContext {
        &self.0.context
    }
    pub fn context(&self) -> Result<SourceContext, SourceError> {
        Ok(self.0.context.clone())
    }
}
/// Field order frees the actual erased future before returning the Source loan.
pub struct ManagedSourceFuture<T> {
    work: ManagedFuture<T>,
    _lifetime: SourceLifetime,
}
impl<T> ManagedSourceFuture<T> {
    pub fn new<F>(lifetime: SourceLifetime, future: F) -> Result<Self, NativeAllocationError>
    where
        F: Future<Output = T> + Send + 'static,
    {
        let context = &lifetime.0.context;
        let work = ManagedFuture::try_new(context, future)?;
        Ok(Self {
            work,
            _lifetime: lifetime,
        })
    }
}
impl<T> Future for ManagedSourceFuture<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        Pin::new(&mut self.get_mut().work).poll(cx)
    }
}
/// Dispatch remains outside the work Box and its enclosing Source owner.
pub struct ManagedDispatchFuture<T> {
    work: ManagedSourceFuture<T>,
    _dispatch: ActiveDispatch,
}
impl<T> ManagedDispatchFuture<T> {
    pub fn new(work: ManagedSourceFuture<T>, dispatch: ActiveDispatch) -> Self {
        Self {
            work,
            _dispatch: dispatch,
        }
    }
}
impl<T> Future for ManagedDispatchFuture<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        Pin::new(&mut self.get_mut().work).poll(cx)
    }
}

#[cfg(test)]
mod tests;
