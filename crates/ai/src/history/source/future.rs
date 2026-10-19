//! By-value future ownership refunds its box only after physical deallocation.
use super::{NativeAllocation, SourceContext};
use crate::model::json::deferred::{JsonDecodeError, NativeAllocationError};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// An actual native future box and its original source credit. Do not box this
/// wrapper again to satisfy an old BoxFuture port: return it by value instead.
/// Original producer/dispatch loans belong to the outer operation AFTER this
/// owner, so they cannot return while its future allocation still exists.
pub struct ManagedFuture<T> {
    future: Pin<Box<dyn Future<Output = T> + Send + 'static>>,
    _allocation: NativeAllocation,
}
impl<T> ManagedFuture<T> {
    pub fn new<F>(context: &SourceContext, future: F) -> Result<Self, JsonDecodeError>
    where
        F: Future<Output = T> + Send + 'static,
    {
        Self::try_new(context, future).map_err(Into::into)
    }
    pub fn try_new<F>(context: &SourceContext, future: F) -> Result<Self, NativeAllocationError>
    where
        F: Future<Output = T> + Send + 'static,
    {
        let allocation = context.reserve_requested(std::alloc::Layout::new::<F>())?;
        Ok(Self {
            future: Box::pin(future),
            _allocation: allocation,
        })
    }
}
impl<T> Future for ManagedFuture<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.get_mut().future.as_mut().poll(cx)
    }
}
impl<T> std::fmt::Debug for ManagedFuture<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedFuture")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::source::SourceProducer,
        model::retention::{RetentionAuthority, document::DocumentRetention},
    };
    use futures::{FutureExt, task::noop_waker};
    #[test]
    fn inline_future_admission_keeps_original_context_and_refuses_closed_source() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
        let lease = producer.capture().unwrap();
        let context = lease.context().unwrap();
        let baseline = context.ledger().live_requested_bytes();
        let data = [7u8; 4096];
        let future =
            ManagedFuture::try_new(&context, async move { std::hint::black_box(data)[0] }).unwrap();
        assert!(context.ledger().live_requested_bytes() >= baseline + data.len());
        drop(lease);
        assert!(matches!(
            ManagedFuture::try_new(&context, async {}),
            Err(NativeAllocationError::Retention(_))
        ));
        assert!(context.ledger().live_requested_bytes() >= baseline + data.len());
        drop(future);
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
    }

    #[test]
    fn future_box_stays_paid_after_ready_and_through_cancelled_capture_destruction() {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
        let lease = producer.capture().unwrap();
        let context = lease.context().unwrap();
        let baseline = context.ledger().live_requested_bytes();
        let data = [7u8; 4096];
        let mut future =
            ManagedFuture::new(&context, async move { std::hint::black_box(data)[0] }).unwrap();
        assert!(context.ledger().live_requested_bytes() >= baseline + data.len());
        assert_eq!(
            future.poll_unpin(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(7)
        );
        assert!(context.ledger().live_requested_bytes() >= baseline + data.len());
        drop(future);
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
        struct Capture {
            context: SourceContext,
            baseline: usize,
            data: [u8; 4096],
        }
        impl Drop for Capture {
            fn drop(&mut self) {
                assert!(
                    self.context.ledger().live_requested_bytes() >= self.baseline + self.data.len()
                );
            }
        }
        let capture = Capture {
            context: context.clone(),
            baseline,
            data,
        };
        let future = ManagedFuture::new(&context, async move {
            futures::future::pending::<()>().await;
            drop(capture);
        })
        .unwrap();
        drop(future);
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
        drop(lease);
        assert!(ManagedFuture::new(&context, async {}).is_err());
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
    }
}
