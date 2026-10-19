//! Original capability return across publication, cancellation and stale delivery.
use super::*;

#[test]
fn original_writer_capabilities_survive_latest_replacement_and_cancelled_native_requests() {
    use crate::history::writing::{ManagedWriteIntent, WriterCapabilityError, WriterProducer};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let producer = WriterProducer::new(
        roles.take_writer_latest().unwrap(),
        roles.take_writer_active().unwrap(),
        roles.take_writer().unwrap(),
    )
    .unwrap();
    live.replace(snapshot(&operation, "old physical payload"))
        .unwrap();
    let mut latest = producer
        .latest(&live, None, ManagedWriteIntent::Snapshot)
        .unwrap();
    let old_bytes = scope.retained_and_pending_bytes().unwrap();
    let active = latest.capture().unwrap();
    assert!(matches!(
        latest.capture(),
        Err(WriterCapabilityError::Unavailable)
    ));
    live.replace(snapshot(&operation, "new physical payload"))
        .unwrap();
    latest = producer
        .latest(&live, Some(&mut latest), ManagedWriteIntent::Snapshot)
        .unwrap();
    assert!(scope.retained_and_pending_bytes().unwrap() > old_bytes);
    active
        .write(|_, snapshot, _| {
            let RowBody::Agent(text) = snapshot.rows()[0].view() else {
                panic!("original active")
            };
            assert_eq!(&*text.read(), "old physical payload");
            assert!(matches!(
                latest.capture(),
                Err(WriterCapabilityError::Unavailable)
            ));
            Ok(())
        })
        .unwrap();
    let after_old = scope.retained_and_pending_bytes().unwrap();
    let next = latest.capture().unwrap();
    // Cancellation before adapter polling still returns the same originals.
    drop(next);
    latest
        .capture()
        .unwrap()
        .write(|_, snapshot, _| {
            let RowBody::Agent(text) = snapshot.rows()[0].view() else {
                panic!("latest active")
            };
            assert_eq!(&*text.read(), "new physical payload");
            Ok(())
        })
        .unwrap();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), after_old);
    drop(latest);
    let replacement = producer
        .latest(&live, None, ManagedWriteIntent::Snapshot)
        .unwrap();
    drop(replacement);
    operation.begin_phase().unwrap();
    live.clear();
    drop(producer);
    assert!(scope.retained_and_pending_bytes().unwrap() < old_bytes);
}

#[test]
fn original_reader_returns_after_unpolled_cancel_panic_failure_and_stale_delivery() {
    use crate::history::{HistoryOrigin, NativeMaterialized, ReaderSource, ReaderSourceError};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut live = roles.take_live().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    let request = source.capture().unwrap();
    assert!(matches!(
        source.capture(),
        Err(ReaderSourceError::Unavailable)
    ));
    drop(request);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
    let request = source.capture().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        request.read(|ledger| {
            let _pending = native_chat(ledger, "pending before callback panic");
            panic!("native callback panic");
        })
    }));
    assert!(panic.is_err());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
    let failed = source
        .capture()
        .unwrap()
        .fail(crate::storage::AgentStorageError::Interrupted);
    assert!(failed.error().is_some());
    failed.release().unwrap();
    let stale = source.capture().unwrap().read(|ledger| {
        Ok(NativeMaterialized::new(
            native_chat(ledger, "stale actual payload"),
            HistoryOrigin::Version2,
            None,
            0,
        ))
    });
    assert!(stale.snapshot().is_some());
    assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
    drop(stale);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
    let ready = source.capture().unwrap().read(|ledger| {
        Ok(NativeMaterialized::new(
            native_chat(ledger, "original live payload"),
            HistoryOrigin::Version2,
            None,
            0,
        ))
    });
    let (_, mut foreign) = DocumentRetention::new(&authority).unwrap();
    let failure = ready
        .install_into(&mut foreign.take_live().unwrap())
        .unwrap_err();
    let (ready, _) = failure.into_parts();
    assert!(ready.snapshot().is_some());
    ready.install_into(&mut live).unwrap();
    assert!(live.snapshot().is_some());
    source
        .capture()
        .unwrap()
        .fail(crate::storage::AgentStorageError::Interrupted)
        .release()
        .unwrap();
    assert!(live.snapshot().is_some());
    live.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}
