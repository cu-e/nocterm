use super::*;
use gpui::TestAppContext;
use nocterm_session::recording::PhysicalPrefix;
use nocterm_session::testing::recording::{ScriptedOutputLog, ScriptedOutputLogFactory};
use std::path::PathBuf;

#[gpui::test]
async fn append_unicode_chunks_drains_and_seals(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let log = Arc::new(ScriptedOutputLog::new(PathBuf::from("/test.log")));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );

    let (entity, handle) = cx.update(|cx| {
        let mut handle_slot = None;
        let entity = cx.new(|cx| {
            let (persisted, handle) = AppendPersisted::new(
                factory.clone(),
                RecordingOptions {
                    directory: None,
                    max_file_mib: 10,
                },
                None,
                cx,
            );
            handle_slot = Some(handle);
            persisted
        });
        (entity, handle_slot.unwrap())
    });

    log.enqueue_append(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(PhysicalPrefix::new(1, 4, 0, false, false), None),
    );
    log.enqueue_seal(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(PhysicalPrefix::new(1, 4, 0, true, true), None),
    );

    cx.update(|cx| {
        handle.output("test".as_bytes(), cx);
        handle.stop(cx);
    });

    cx.run_until_parked();

    let status = cx.update(|cx| entity.read(cx).status.clone());
    assert_eq!(status.bytes, 4);
    assert!(status.finished);
    assert!(status.error.is_none());
}

#[gpui::test]
async fn partial_disk_failure_survives_final_seal_and_public_status_clones(
    cx: &mut TestAppContext,
) {
    use std::error::Error;
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let log = Arc::new(ScriptedOutputLog::new(PathBuf::from("/partial.log")));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );
    let original =
        RecordingError::append("/partial.log", 2, std::io::ErrorKind::StorageFull.into());
    log.enqueue_append(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(
            PhysicalPrefix::new(0, 2, 2, false, false),
            Some(original.clone()),
        ),
    );
    log.enqueue_seal(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(
            PhysicalPrefix::new(0, 2, 2, false, true),
            Some(original.clone()),
        ),
    );
    let (_entity, handle) = cx.update(|cx| {
        let mut slot = None;
        let entity = cx.new(|cx| {
            let (persisted, handle) = AppendPersisted::new(
                factory.clone(),
                RecordingOptions {
                    directory: None,
                    max_file_mib: 10,
                },
                None,
                cx,
            );
            slot = Some(handle);
            persisted
        });
        (entity, slot.unwrap())
    });
    cx.update(|cx| {
        handle.output(b"bytes", cx);
        handle.stop(cx);
    });
    cx.run_until_parked();
    let status = handle.status();
    assert_eq!(status.bytes(), 2);
    assert!(status.finished());
    assert_eq!(log.seal_count(), 1);
    let native = original
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    for snapshot in [status, handle.status()] {
        let shared = snapshot
            .error()
            .unwrap()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(shared.kind(), std::io::ErrorKind::StorageFull);
        assert!(std::ptr::eq(native, shared));
    }
}

#[gpui::test]
async fn public_append_handles_follow_physical_status_while_operations_are_held(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let path = PathBuf::from("/held.log");
    let log = Arc::new(ScriptedOutputLog::new(path.clone()));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );
    let (_entity, handle) = cx.update(|cx| {
        let mut slot = None;
        let entity = cx.new(|cx| {
            let (persisted, handle) = AppendPersisted::new(
                factory.clone(),
                RecordingOptions {
                    directory: None,
                    max_file_mib: 10,
                },
                None,
                cx,
            );
            slot = Some(handle);
            persisted
        });
        (entity, slot.unwrap())
    });
    let clone = handle.clone();
    cx.run_until_parked();
    let created = handle.status();
    assert_eq!(created.path(), Some(&path));
    assert_eq!(created.bytes(), 0);
    assert!(!created.finished());
    assert!(created.error().is_none());
    let append_gate = nocterm_session::testing::Gate::pending();
    log.enqueue_append(
        append_gate.clone(),
        AppendOutcome::new(PhysicalPrefix::new(1, 3, 0, false, false), None),
    );
    cx.update(|cx| handle.output(b"abc", cx));
    cx.run_until_parked();
    assert_eq!(log.requests().len(), 1);
    assert_eq!(handle.status(), created);
    append_gate.release();
    cx.run_until_parked();
    let appended = clone.status();
    assert_eq!(appended.bytes(), 3);
    assert_eq!(appended.path(), Some(&path));
    assert!(!appended.finished());
    assert!(appended.error().is_none());
    assert_eq!(handle.status(), appended);
    assert_eq!(
        created.bytes(),
        0,
        "an earlier immutable snapshot stays unchanged"
    );
    let seal_gate = nocterm_session::testing::Gate::pending();
    log.enqueue_seal(
        seal_gate.clone(),
        AppendOutcome::new(PhysicalPrefix::new(1, 3, 0, true, true), None),
    );
    cx.update(|cx| clone.stop(cx));
    cx.run_until_parked();
    assert_eq!(log.seal_count(), 1);
    assert_eq!(handle.status(), appended);
    seal_gate.release();
    cx.run_until_parked();
    let sealed = handle.status();
    assert!(sealed.finished());
    assert_eq!(sealed.bytes(), 3);
    assert_eq!(sealed.path(), Some(&path));
    assert!(sealed.error().is_none());
    assert_eq!(clone.status(), sealed);
}

#[gpui::test]
async fn public_append_handles_keep_draining_after_repeated_idle_returns(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let log = Arc::new(ScriptedOutputLog::new(PathBuf::from("/busy.log")));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );
    let (_entity, handle) = cx.update(|cx| {
        let mut slot = None;
        let entity = cx.new(|cx| {
            let (persisted, handle) = AppendPersisted::new(
                factory.clone(),
                RecordingOptions {
                    directory: None,
                    max_file_mib: 10,
                },
                None,
                cx,
            );
            slot = Some(handle);
            persisted
        });
        (entity, slot.unwrap())
    });
    let clone = handle.clone();
    cx.run_until_parked();
    for round in 0..4 {
        for step in 1..=8 {
            let count = round * 8 + step;
            log.enqueue_append(
                nocterm_session::testing::Gate::open(),
                AppendOutcome::new(PhysicalPrefix::new(count, count, 0, false, false), None),
            );
        }
        cx.update(|cx| {
            for _ in 0..8 {
                handle.output(b"x", cx);
            }
        });
        cx.run_until_parked();
        assert_eq!(clone.status().bytes(), (round + 1) * 8);
        assert_eq!(log.requests().len() as u64, (round + 1) * 8);
        assert!(!clone.status().finished());
    }
    log.enqueue_seal(
        nocterm_session::testing::Gate::open(),
        AppendOutcome::new(PhysicalPrefix::new(32, 32, 0, true, true), None),
    );
    cx.update(|cx| clone.stop(cx));
    cx.run_until_parked();
    assert_eq!(log.seal_count(), 1);
    assert!(handle.status().finished());
    assert_eq!(handle.status().bytes(), 32);
    assert!(handle.status().error().is_none());
}

#[gpui::test]
async fn recording_registry_handles_publish_held_progress_and_final_error(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedOutputLogFactory::default());
    let path = PathBuf::from("/registered.log");
    let log = Arc::new(ScriptedOutputLog::new(path.clone()));
    factory.enqueue(
        nocterm_session::testing::Gate::open(),
        Ok(log.clone() as Arc<dyn OutputLog>),
    );
    let registry = RecordingRegistry::new(factory);
    let handle = cx.update(|cx| {
        registry.open(
            RecordingOptions {
                directory: None,
                max_file_mib: 10,
            },
            cx,
        )
    });
    let clone = handle.clone();
    cx.run_until_parked();
    let created = clone.status();
    assert_eq!(created.path(), Some(&path));
    assert_eq!(created.bytes(), 0);
    assert!(!created.finished());
    assert!(created.error().is_none());
    let append_gate = nocterm_session::testing::Gate::pending();
    log.enqueue_append(
        append_gate.clone(),
        AppendOutcome::new(PhysicalPrefix::new(1, 3, 0, false, false), None),
    );
    cx.update(|cx| handle.output(b"abc", cx));
    cx.run_until_parked();
    assert_eq!(log.requests().len(), 1);
    assert_eq!(handle.status(), created);
    append_gate.release();
    cx.run_until_parked();
    let appended = clone.status();
    assert_eq!(appended.bytes(), 3);
    assert_eq!(appended.path(), Some(&path));
    assert!(!appended.finished());
    assert!(appended.error().is_none());
    let seal_gate = nocterm_session::testing::Gate::pending();
    let failure = RecordingError::sync(
        path.clone(),
        std::io::Error::from(std::io::ErrorKind::Other),
    );
    log.enqueue_seal(
        seal_gate.clone(),
        AppendOutcome::new(
            PhysicalPrefix::new(1, 3, 0, true, false),
            Some(failure.clone()),
        ),
    );
    cx.update(|cx| clone.stop(cx));
    cx.run_until_parked();
    assert_eq!(log.seal_count(), 1);
    assert_eq!(handle.status(), appended);
    seal_gate.release();
    cx.run_until_parked();
    let finished = handle.status();
    assert!(finished.finished());
    assert_eq!(finished.bytes(), 3);
    assert_eq!(finished.path(), Some(&path));
    assert_eq!(finished.error(), Some(&failure));
    assert_eq!(clone.status(), finished);
    assert!(created.error().is_none());
    assert_eq!(created.bytes(), 0);
}
