//! Value admission and inseparable physical-operation lifetime.
use super::KeyedWriter;
use crate::StoreError;
use futures::future::BoxFuture;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A checked intake leaves both values valid on error. Capture owns everything
/// the physical operation needs until its returned future has been destroyed.
pub trait KeyedValueOwnership<V>: Send + Sync + 'static {
    type Write: Send + 'static;
    type Guard: Send + 'static;
    fn accept(&self, incoming: &mut V, previous: Option<&mut V>) -> Result<(), StoreError>;
    fn capture(&self, latest: &V) -> Result<KeyedCapture<Self::Write, Self::Guard>, StoreError>;
}
#[derive(Clone, Copy, Default)]
pub struct CloneValues;
impl<V: Clone + Send + 'static> KeyedValueOwnership<V> for CloneValues {
    type Write = V;
    type Guard = ();
    fn accept(&self, _: &mut V, _: Option<&mut V>) -> Result<(), StoreError> {
        Ok(())
    }
    fn capture(&self, latest: &V) -> Result<KeyedCapture<V, ()>, StoreError> {
        Ok(KeyedCapture::new(latest.clone(), ()))
    }
}
/// Field order also covers an operation dropped before its factory is called.
pub struct KeyedCapture<W, G> {
    value: W,
    guard: G,
}
impl<W, G> KeyedCapture<W, G> {
    pub fn new(value: W, guard: G) -> Self {
        Self { value, guard }
    }
}
impl<W: Send + 'static, G: Send + 'static> KeyedCapture<W, G> {
    pub(super) fn run<K: Send + 'static>(
        self,
        key: K,
        writer: KeyedWriter<K, W>,
    ) -> BoxFuture<'static, Result<(), StoreError>> {
        let Self { value, guard } = self;
        // The argument/request unwinds before this caller-local guard if the
        // writer factory panics. Once constructed, the wrapper owns both.
        let inner = writer(key, value);
        Box::pin(OwnedWriteFuture {
            inner: Some(inner),
            guard: Some(guard),
        })
    }
}
struct OwnedWriteFuture<G> {
    inner: Option<BoxFuture<'static, Result<(), StoreError>>>,
    guard: Option<G>,
}
// No pinned reference to the guard is exposed; the nested future stays pinned
// in its own Box while this enclosing owner moves.
impl<G> Unpin for OwnedWriteFuture<G> {}
impl<G> Future for OwnedWriteFuture<G> {
    type Output = Result<(), StoreError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = match self.inner.as_mut() {
            Some(inner) => match inner.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result,
            },
            None => return Poll::Ready(Err(StoreError::CompletionDropped)),
        };
        drop(self.inner.take());
        drop(self.guard.take());
        Poll::Ready(result)
    }
}
impl<G> Drop for OwnedWriteFuture<G> {
    fn drop(&mut self) {
        drop(self.inner.take());
        drop(self.guard.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt, future};
    use std::sync::{Arc, Mutex};
    struct Released(&'static str, Arc<Mutex<Vec<&'static str>>>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.1.lock().unwrap().push(self.0);
        }
    }
    fn capture(log: &Arc<Mutex<Vec<&'static str>>>) -> KeyedCapture<Released, Released> {
        KeyedCapture::new(
            Released("payload", log.clone()),
            Released("guard", log.clone()),
        )
    }
    #[test]
    fn unstarted_capture_destroys_payload_before_guard() {
        let log = Arc::new(Mutex::new(Vec::new()));
        drop(capture(&log));
        assert_eq!(*log.lock().unwrap(), ["payload", "guard"]);
    }
    #[test]
    fn cancelled_physical_future_destroys_payload_before_guard() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let writer: KeyedWriter<(), Released> = Arc::new(|_, payload| {
            Box::pin(async move {
                let _payload = payload;
                future::pending::<()>().await;
                Ok(())
            })
        });
        let mut physical = capture(&log).run((), writer);
        assert!(physical.as_mut().now_or_never().is_none());
        assert!(log.lock().unwrap().is_empty());
        drop(physical);
        assert_eq!(*log.lock().unwrap(), ["payload", "guard"]);
    }
    #[test]
    fn completed_success_and_failure_retire_request_before_guard() {
        for failed in [false, true] {
            let log = Arc::new(Mutex::new(Vec::new()));
            let writer: KeyedWriter<(), Released> = Arc::new(move |_, payload| {
                Box::pin(async move {
                    let _payload = payload;
                    if failed {
                        Err(StoreError::PendingWrites)
                    } else {
                        Ok(())
                    }
                })
            });
            assert_eq!(
                futures::executor::block_on(capture(&log).run((), writer)).is_err(),
                failed
            );
            assert_eq!(*log.lock().unwrap(), ["payload", "guard"]);
        }
    }
    #[test]
    fn failed_factory_unwinds_request_before_guard() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let writer: KeyedWriter<(), Released> = Arc::new(|_, _payload| panic!("factory failure"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture(&log).run((), writer)
        }));
        assert!(result.is_err());
        assert_eq!(*log.lock().unwrap(), ["payload", "guard"]);
    }
}
