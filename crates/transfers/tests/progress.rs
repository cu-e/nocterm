//! Pure transfer admission and progress rules, independent of any executor.
use nocterm_session::{Target, testing::InMemoryFs};
use nocterm_transfers::*;
use std::{error::Error as _, io, path::PathBuf, sync::Arc};

fn request() -> Request {
    Request::Upload(UploadRequest {
        sources: vec![PathBuf::from("selected.txt")],
        target: Target::new("u", "h", 22),
        destination: "/dest".into(),
        fs: Arc::new(InMemoryFs::default()),
        collisions: CollisionPolicy::Skip,
    })
}
fn tracker() -> ProgressTracker {
    ProgressTracker::new(
        TransferId::from_bytes([1; 16]).unwrap(),
        &request(),
        StreamLimits::default(),
    )
}
#[test]
fn completion_requires_both_discovery_and_worker_drain() {
    for order in [
        [
            TransferEvent::DiscoveryComplete,
            TransferEvent::WorkersDrained,
        ],
        [
            TransferEvent::WorkersDrained,
            TransferEvent::DiscoveryComplete,
        ],
    ] {
        let mut tracker = tracker();
        let [first, second] = order;
        tracker.apply(first);
        assert!(!tracker.snapshot().state.finished());
        tracker.apply(second);
        assert_eq!(tracker.snapshot().state, TransferState::Completed);
    }
}
#[test]
fn cancellation_stays_live_until_all_cleanup_is_drained() {
    let mut tracker = tracker();
    tracker.apply(TransferEvent::FileStarted);
    tracker.apply(TransferEvent::CancelRequested);
    assert_eq!(tracker.snapshot().state, TransferState::Running);
    tracker.apply(TransferEvent::DiscoveryComplete);
    assert!(!tracker.snapshot().state.finished());
    tracker.apply(TransferEvent::WorkersDrained);
    assert_eq!(tracker.snapshot().state, TransferState::Cancelled);
    tracker.apply(TransferEvent::FileStarted);
    assert_eq!(tracker.snapshot().state, TransferState::Cancelled);
}
#[test]
fn only_acknowledgement_events_advance_bytes() {
    let mut tracker = tracker();
    tracker.apply(TransferEvent::DiscoveredFile { bytes: 32768 });
    tracker.apply(TransferEvent::FileStarted);
    assert_eq!(tracker.snapshot().sent_bytes, 0);
    tracker.apply(TransferEvent::Acknowledged { bytes: 4096 });
    assert_eq!(tracker.snapshot().sent_bytes, 4096);
    assert_eq!(tracker.snapshot().total_bytes, 32768);
}
#[test]
fn diagnostic_details_are_bounded_and_failures_survive_truncation() {
    let mut tracker = tracker();
    for _ in 0..200 {
        tracker.apply(TransferEvent::FileFailed(TransferIssue::new(
            None,
            TransferOperation::Read,
            TransferCause::InvalidName,
        )));
    }
    tracker.apply(TransferEvent::DiscoveryComplete);
    tracker.apply(TransferEvent::WorkersDrained);
    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.errors.len(), 100);
    assert_eq!(snapshot.failed_files, 200);
    assert_eq!(snapshot.state, TransferState::Failed);
}
#[test]
fn notices_do_not_turn_a_committed_file_into_failure() {
    let mut tracker = tracker();
    tracker.apply(TransferEvent::Notice(TransferIssue::new(
        None,
        TransferOperation::Publish,
        TransferCause::Durability(nocterm_session::local_fs::DurabilityWarning::new(
            std::io::Error::other("directory flush"),
        )),
    )));
    tracker.apply(TransferEvent::FileCompleted);
    tracker.apply(TransferEvent::DiscoveryComplete);
    tracker.apply(TransferEvent::WorkersDrained);
    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.state, TransferState::Completed);
    assert_eq!(snapshot.completed_files, 1);
    assert_eq!(snapshot.failed_files, 0);
    assert_eq!(snapshot.errors.len(), 1);
}
#[test]
fn immutable_ids_reject_zero_and_display_without_randomness() {
    assert!(matches!(
        TransferId::from_bytes([0; 16]),
        Err(TransferError::InvalidId)
    ));
    let id = TransferId::from_bytes([1; 16]).unwrap();
    assert_eq!(id.to_string(), "01010101-0101-0101-0101-010101010101");
    assert_eq!(id.bytes(), [1; 16]);
}

#[test]
fn directory_acknowledgements_remain_distinct_from_files_after_every_terminal_outcome() {
    for state in [
        TransferState::Completed,
        TransferState::Failed,
        TransferState::Cancelled,
    ] {
        let mut tracker = tracker();
        tracker.apply(TransferEvent::DirectoryPrepared);
        match state {
            TransferState::Failed => tracker.apply(TransferEvent::FileFailed(TransferIssue::new(
                None,
                TransferOperation::Discover,
                TransferCause::InvalidName,
            ))),
            TransferState::Cancelled => tracker.apply(TransferEvent::CancelRequested),
            _ => {}
        }
        tracker.apply(TransferEvent::DiscoveryComplete);
        tracker.apply(TransferEvent::WorkersDrained);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.state, state);
        assert_eq!(snapshot.prepared_directories, 1);
        assert_eq!(snapshot.discovered_files, 0);
        assert_eq!(snapshot.completed_files, 0);
        assert_eq!(snapshot.sent_bytes, 0);
        tracker.apply(TransferEvent::DirectoryPrepared);
        assert_eq!(
            tracker.snapshot(),
            snapshot,
            "terminal progress is immutable"
        );
    }
}

#[test]
fn runtime_failure_snapshot_keeps_native_leaf_and_shared_notice_identity() {
    let cause = TransferCause::runtime(io::Error::new(
        io::ErrorKind::StorageFull,
        "worker storage full",
    ));
    let anchor = cause.clone();
    let original = anchor
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    let issue = TransferIssue::new(
        Some(TransferPath::Remote("/dest/file".into())),
        TransferOperation::Runtime,
        cause,
    );
    let mut progress = tracker();
    progress.apply(TransferEvent::DiscoveredFile { bytes: 32 });
    progress.apply(TransferEvent::Acknowledged { bytes: 8 });
    progress.apply(TransferEvent::FileFailed(issue.clone()));
    progress.apply(TransferEvent::DiscoveryComplete);
    progress.apply(TransferEvent::WorkersDrained);
    let snapshot = progress.snapshot();
    let cloned = snapshot.clone();
    assert_eq!(snapshot.state, TransferState::Failed);
    assert_eq!(snapshot.sent_bytes, 8);
    assert_eq!(snapshot.total_bytes, 32);
    assert_eq!(snapshot.failed_files, 1);
    assert_eq!(snapshot.errors[0], issue);
    assert_eq!(cloned.errors[0], issue);
    assert_eq!(
        issue.path(),
        Some(&TransferPath::Remote("/dest/file".into()))
    );
    let cause = snapshot.errors[0].source().unwrap();
    assert!(cause.downcast_ref::<TransferCause>().is_some());
    let leaf = cause
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert!(std::ptr::eq(original, leaf));
    assert_eq!(leaf.kind(), io::ErrorKind::StorageFull);
    assert_eq!(
        issue.to_string(),
        "/dest/file: runtime: Transfer runtime failed: worker storage full"
    );
    progress.apply(TransferEvent::Acknowledged { bytes: 7 });
    assert_eq!(
        progress.snapshot(),
        snapshot,
        "terminal accounting is unchanged"
    );
}
#[test]
fn runtime_issue_retains_concrete_wrapper_before_its_native_child() {
    let backend = nocterm_core::PersistError::Io {
        path: "transfer-state".into(),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "worker refused"),
    };
    let description = backend.to_string();
    let cause = TransferCause::runtime(backend);
    let cloned = cause.clone();
    let original = cause.source().unwrap().source().unwrap();
    assert!(
        original
            .downcast_ref::<nocterm_core::PersistError>()
            .is_some()
    );
    assert!(std::ptr::eq(
        original,
        cloned.source().unwrap().source().unwrap()
    ));
    let native = original
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
    let issue = TransferIssue::new(None, TransferOperation::Runtime, cloned);
    assert_eq!(
        issue.to_string(),
        format!("runtime: Transfer runtime failed: {description}")
    );
    assert!(
        issue
            .source()
            .unwrap()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<nocterm_core::PersistError>()
            .is_some()
    );
}
