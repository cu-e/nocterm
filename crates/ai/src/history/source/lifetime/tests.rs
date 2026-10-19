//! The original role stays busy through driver, ready and unpolled future owners.
use super::*;
use crate::{
    history::source::SourceProducer,
    model::{
        allocations::shared_inline_bytes,
        retention::{RetentionAuthority, document::DocumentRetention},
    },
};
fn fixture() -> SourceProducer {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    SourceProducer::new(roles.take_source().unwrap()).unwrap()
}
#[test]
fn original_source_waits_for_both_driver_and_ready_connect_future_drop_orders() {
    use futures::{FutureExt, task::noop_waker};
    for driver_first in [false, true] {
        let producer = fixture();
        let original = producer.capture().unwrap();
        let lifetime = original.into_lifetime().unwrap();
        let driver = lifetime.clone();
        let mut future = ManagedSourceFuture::new(lifetime, async { 42 }).unwrap();
        assert_eq!(
            future.poll_unpin(&mut Context::from_waker(&noop_waker())),
            Poll::Ready(42)
        );
        assert!(producer.capture().is_err());
        if driver_first {
            drop(driver);
            assert!(producer.capture().is_err());
            drop(future);
        } else {
            drop(future);
            assert!(producer.capture().is_err());
            drop(driver);
        }
        assert!(producer.capture().is_ok());
    }
}
#[test]
fn exact_phase_independent_shared_header_retires_with_the_original_lifetime() {
    let producer = fixture();
    let original = producer.capture().unwrap();
    let context = original.context().unwrap();
    let before = context.0.scope.retained_and_pending_bytes().unwrap();
    let lifetime = original.into_lifetime().unwrap();
    let during = context.0.scope.retained_and_pending_bytes().unwrap();
    assert_eq!(during - before, shared_inline_bytes::<SourceLeaseBody>());
    let alias = lifetime.clone();
    drop(lifetime);
    assert_eq!(
        context.0.scope.retained_and_pending_bytes().unwrap(),
        during
    );
    drop(alias);
    assert_eq!(
        context.0.scope.retained_and_pending_bytes().unwrap(),
        before
    );
    assert!(producer.capture().is_ok());
}
#[test]
fn never_polled_and_panicking_future_captures_do_not_release_the_original_role_early() {
    struct Capture {
        producer: SourceProducer,
        panic: bool,
        bytes: [u8; 4096],
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            assert!(self.producer.capture().is_err());
            std::hint::black_box(&self.bytes);
            assert!(!self.panic, "intentional destructor unwind");
        }
    }
    for panic in [false, true] {
        let producer = fixture();
        let lifetime = producer.capture().unwrap().into_lifetime().unwrap();
        let capture = Capture {
            producer: producer.clone(),
            panic,
            bytes: [7; 4096],
        };
        let future = ManagedSourceFuture::new(lifetime, async move {
            futures::future::pending::<()>().await;
            drop(capture);
        })
        .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(future)));
        assert_eq!(result.is_err(), panic);
        assert!(producer.capture().is_ok());
    }
}
