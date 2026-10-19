//! Original reader availability follows native task destruction and cancellation.
use super::*;
use crate::{
    history::ReaderSource,
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn fixture() -> (
    ReaderSource,
    ReaderDocumentId,
    crate::model::retention::document::ConstructionRole,
) {
    let authority = RetentionAuthority::new(crate::history::MAX_RESIDENT_BYTES).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let foreground = roles.take_foreground().unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let value = "original document";
    let allocation = ledger.reserve(value.len()).unwrap();
    let id = ReaderDocumentId::from_native(NativeOwned::from_parts(value.to_owned(), allocation));
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    (source, id, foreground)
}
struct Task {
    id: ReaderDocumentId,
    source: ReaderSource,
    dropped: Arc<AtomicBool>,
    pending: bool,
    panic_ready: bool,
}
impl ReaderMaterializer for Task {
    fn poll_ready(
        &mut self,
        _: &mut Context<'_>,
        _: &DecodeLedger,
    ) -> Poll<Result<(), AgentStorageError>> {
        assert!(!self.panic_ready, "scripted readiness panic");
        if self.pending {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }
    fn materialize(&mut self, _: &DecodeLedger) -> Result<NativeMaterialized, AgentStorageError> {
        assert_eq!(self.id.as_str(), "original document");
        Err(AgentStorageError::Interrupted)
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        assert!(matches!(
            self.source.capture(),
            Err(super::super::super::ReaderSourceError::Unavailable)
        ));
        self.dropped.store(true, Ordering::Release);
    }
}
fn staged(
    source: &ReaderSource,
    id: ReaderDocumentId,
    pending: bool,
    panic_ready: bool,
) -> (ManagedReaderFuture, Arc<AtomicBool>) {
    let dropped = Arc::new(AtomicBool::new(false));
    let probe = dropped.clone();
    let original = source.clone();
    let future = source
        .capture()
        .unwrap()
        .prepare_native(id, move |id, _| {
            Ok(Task {
                id,
                source: original,
                dropped: probe,
                pending,
                panic_ready,
            })
        })
        .unwrap();
    (future, dropped)
}
#[test]
fn unpolled_and_pending_tasks_die_before_original_reader_returns() {
    use futures::FutureExt as _;
    for pending in [false, true] {
        let (source, id, _foreground) = fixture();
        let (mut future, dropped) = staged(&source, id, pending, false);
        if pending {
            assert!(Pin::new(&mut future).now_or_never().is_none());
        }
        assert!(matches!(
            source.capture(),
            Err(super::super::super::ReaderSourceError::Unavailable)
        ));
        drop(future);
        assert!(dropped.load(Ordering::Acquire));
        drop(source.capture().unwrap());
    }
}
#[test]
fn completed_task_dies_before_delivery_and_same_reader_recycles() {
    let (source, id, _foreground) = fixture();
    let (future, dropped) = staged(&source, id, false, false);
    let delivery = futures::executor::block_on(future);
    assert!(dropped.load(Ordering::Acquire));
    assert!(delivery.error().is_some());
    assert!(source.capture().is_err());
    drop(delivery);
    drop(source.capture().unwrap());
}
#[test]
fn readiness_panic_and_factory_panic_return_same_original_reader() {
    let (source, id, _foreground) = fixture();
    let (mut future, dropped) = staged(&source, id, false, true);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use futures::FutureExt as _;
        let _ = Pin::new(&mut future).now_or_never();
    }));
    assert!(panic.is_err());
    drop(future);
    assert!(dropped.load(Ordering::Acquire));
    drop(source.capture().unwrap());

    let (source, id, _foreground) = fixture();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = source
            .capture()
            .unwrap()
            .prepare_native::<Task>(id, |_, _| panic!("scripted factory panic"));
    }));
    assert!(panic.is_err());
    drop(source.capture().unwrap());
}
#[test]
fn foreign_document_identity_refuses_without_factory_and_keeps_original_request() {
    let (source, _, _foreground) = fixture();
    let (_foreign_source, foreign, _foreign_foreground) = fixture();
    let called = AtomicBool::new(false);
    let result = source
        .capture()
        .unwrap()
        .prepare_native::<Task>(foreign, |_, _| {
            called.store(true, Ordering::Release);
            panic!("foreign factory")
        });
    let failure = match result {
        Err(error) => error,
        Ok(_) => panic!("foreign identity"),
    };
    assert!(!called.load(Ordering::Acquire));
    assert!(source.capture().is_err());
    drop(failure);
    drop(source.capture().unwrap());
}

#[test]
fn paid_identity_survives_foreground_retirement_while_original_gate_waits() {
    use futures::FutureExt as _;
    let (source, id, foreground) = fixture();
    let gate = crate::history::HistoryGate::default();
    let held = futures::executor::block_on(gate.acquire());
    let original = source.clone();
    let mut waiting = Box::pin(async move {
        let permit = gate.acquire().await;
        let (future, dropped) = staged(&original, id, false, false);
        (future.await, permit, dropped)
    });
    assert!(waiting.as_mut().now_or_never().is_none());
    // The original reader has not been captured before the actual shared gate.
    drop(source.capture().unwrap());
    drop(foreground);
    drop(held);
    let (delivery, permit, dropped) = futures::executor::block_on(waiting);
    assert!(dropped.load(Ordering::Acquire));
    assert!(delivery.error().is_some());
    drop(delivery);
    drop(permit);
    drop(source.capture().unwrap());
}
