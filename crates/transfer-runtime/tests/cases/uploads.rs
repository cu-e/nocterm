use super::*;
use nocterm_session::{
    FsError, RemoteFs,
    testing::{FsOperation, Gate},
};
use nocterm_transfers::{
    CollisionPolicy, TransferCause, TransferError, TransferOperation, TransferPath,
    TransferService, TransferState, UploadRequest,
};
use std::sync::Arc;

#[test]
fn many_files_stream_with_four_writers_and_bounded_acknowledgement_batches() {
    let fixture = Fixture::new();
    let mut sources = Vec::new();
    for index in 0..100 {
        let source = native(&format!("input/file-{index}"));
        fixture
            .local
            .seed_native_file(&source, vec![index as u8; 65539])
            .unwrap();
        sources.push(source);
    }
    let id = fixture.upload(sources, CollisionPolicy::Skip);
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.completed_files, 100);
    assert_eq!(progress.sent_bytes, 100 * 65539);
    let activity = fixture.remote.activity();
    assert!(activity.peak_uploads <= 4);
    assert!(activity.max_write_chunk <= 32768);
    assert!(activity.max_write_batch <= 8);
    assert_eq!(activity.acknowledged_bytes as u64, progress.sent_bytes);
    assert_eq!(
        fixture.remote.bytes("/dest/file-99").unwrap(),
        vec![99; 65539]
    );
    assert_eq!(activity.live_uploads, 0);
}
#[test]
fn collision_choices_preserve_existing_bytes_and_rename_extensions() {
    for policy in [
        CollisionPolicy::Skip,
        CollisionPolicy::Rename,
        CollisionPolicy::Replace,
    ] {
        let fixture = Fixture::new();
        let source = native("input/file.txt");
        fixture
            .local
            .seed_native_file(&source, b"incoming".to_vec())
            .unwrap();
        fixture
            .remote
            .seed_file("/dest/file.txt", b"external")
            .unwrap();
        let progress = fixture.finished(fixture.upload(vec![source], policy));
        assert_eq!(progress.state, TransferState::Completed);
        match policy {
            CollisionPolicy::Skip => {
                assert_eq!(progress.skipped_files, 1);
                assert_eq!(fixture.remote.bytes("/dest/file.txt").unwrap(), b"external");
            }
            CollisionPolicy::Rename => {
                assert_eq!(fixture.remote.bytes("/dest/file.txt").unwrap(), b"external");
                assert_eq!(
                    fixture.remote.bytes("/dest/file (1).txt").unwrap(),
                    b"incoming"
                );
            }
            CollisionPolicy::Replace => {
                assert_eq!(fixture.remote.bytes("/dest/file.txt").unwrap(), b"incoming")
            }
        }
    }
}
#[test]
fn late_create_collision_reuses_same_acknowledged_stage_without_rereading_source() {
    let fixture = Fixture::new();
    let source = native("input/file.txt");
    fixture
        .local
        .seed_native_file(&source, b"incoming".to_vec())
        .unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Finish, "/dest/file.txt", gate.clone());
    let id = fixture.upload(vec![source.clone()], CollisionPolicy::Rename);
    wait(|| {
        fixture
            .runtime
            .snapshot()
            .iter()
            .any(|progress| progress.id == id && progress.sent_bytes == 8)
    });
    fixture
        .remote
        .seed_file("/dest/file.txt", b"external")
        .unwrap();
    fixture
        .local
        .seed_native_file(&source, b"must-not-be-read-again".to_vec())
        .unwrap();
    gate.release();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.sent_bytes, 8);
    assert_eq!(fixture.remote.bytes("/dest/file.txt").unwrap(), b"external");
    assert_eq!(
        fixture.remote.bytes("/dest/file (1).txt").unwrap(),
        b"incoming"
    );
    assert_eq!(fixture.remote.activity().acknowledged_bytes, 8);
    assert_eq!(fixture.remote.activity().completed_uploads, 1);
}
#[test]
fn cancellation_drains_stalled_acknowledgements_and_never_publishes_partial_bytes() {
    let fixture = Fixture::new();
    let source = native("input/file");
    fixture
        .local
        .seed_native_file(&source, vec![1; 65536])
        .unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Write, "/dest/file", Gate::pending());
    let id = fixture.upload(vec![source], CollisionPolicy::Skip);
    wait(|| fixture.remote.activity().live_uploads == 1);
    fixture.runtime.cancel(id).unwrap();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Cancelled);
    assert_eq!(progress.sent_bytes, 0);
    assert!(fixture.remote.bytes("/dest/file").is_err());
    assert_eq!(fixture.remote.activity().live_uploads, 0);
    assert_eq!(fixture.local.activity().live_downloads, 0);
}
#[test]
fn failed_ack_is_typed_and_retry_keeps_target_destination_and_new_remote_port() {
    let fixture = Fixture::new();
    let source = native("input/file");
    fixture
        .local
        .seed_native_file(&source, b"data".to_vec())
        .unwrap();
    fixture.remote.fail_next(
        FsOperation::Write,
        "/dest/file",
        FsError::PermissionDenied {
            path: "/dest/file".into(),
        },
    );
    let first = fixture.upload(vec![source], CollisionPolicy::Skip);
    let failed = fixture.finished(first);
    assert_eq!(failed.state, TransferState::Failed);
    assert_eq!(failed.sent_bytes, 0);
    assert_eq!(failed.errors.len(), 1);
    let issue = &failed.errors[0];
    assert_eq!(issue.operation(), TransferOperation::Write);
    assert_eq!(
        issue.path(),
        Some(&TransferPath::Remote("/dest/file".into()))
    );
    assert!(
        matches!(issue.cause(), TransferCause::Remote(FsError::PermissionDenied { path }) if path == "/dest/file")
    );
    assert_eq!(
        failed.clone().errors[0],
        *issue,
        "snapshot cloning preserves the injected diagnostic identity"
    );
    assert!(fixture.remote.bytes("/dest/file").is_err());
    let replacement = Arc::new(nocterm_session::testing::InMemoryFs::default());
    futures::executor::block_on(replacement.create_dir("/dest")).unwrap();
    let retried = fixture.runtime.retry(first, replacement.clone()).unwrap();
    let completed = fixture.finished(retried);
    assert_eq!(completed.state, TransferState::Completed);
    assert_eq!(completed.target, failed.target);
    assert_eq!(completed.destination, failed.destination);
    assert_eq!(replacement.bytes("/dest/file").unwrap(), b"data");
}
#[test]
fn bounded_queue_reports_backpressure_and_queued_cancellation_opens_nothing() {
    let fixture = Fixture::new();
    let active = native("input/active");
    fixture
        .local
        .seed_native_file(&active, b"active".to_vec())
        .unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Write, "/dest/active", Gate::pending());
    let active_id = fixture.upload(vec![active], CollisionPolicy::Skip);
    wait(|| fixture.remote.activity().live_uploads == 1);
    let mut queued = Vec::new();
    for index in 0..8 {
        let source = native(&format!("input/queued-{index}"));
        fixture
            .local
            .seed_native_file(&source, b"queued".to_vec())
            .unwrap();
        queued.push(fixture.upload(vec![source], CollisionPolicy::Skip));
    }
    let source = native("input/overflow");
    fixture
        .local
        .seed_native_file(&source, b"overflow".to_vec())
        .unwrap();
    let result = fixture.runtime.enqueue(UploadRequest {
        sources: vec![source],
        target: nocterm_session::Target::new("u", "h", 22),
        destination: "/dest".into(),
        fs: fixture.remote.clone(),
        collisions: CollisionPolicy::Skip,
    });
    assert!(matches!(result, Err(TransferError::Full)));
    fixture.runtime.cancel(queued[0]).unwrap();
    fixture.runtime.cancel(active_id).unwrap();
    assert_eq!(fixture.finished(queued[0]).state, TransferState::Cancelled);
    for id in queued.into_iter().skip(1) {
        fixture.finished(id);
    }
    assert!(fixture.remote.bytes("/dest/queued-0").is_err());
    assert_eq!(fixture.remote.activity().completed_uploads, 7);
}
#[test]
fn same_basename_sources_reserve_distinct_rename_destinations() {
    let fixture = Fixture::new();
    let first = native("input/a/same.txt");
    let second = native("input/b/same.txt");
    fixture
        .local
        .seed_native_file(&first, b"first".to_vec())
        .unwrap();
    fixture
        .local
        .seed_native_file(&second, b"second".to_vec())
        .unwrap();
    let progress = fixture.finished(fixture.upload(vec![first, second], CollisionPolicy::Rename));
    assert_eq!(progress.completed_files, 2);
    let mut bytes = [
        fixture.remote.bytes("/dest/same.txt").unwrap(),
        fixture.remote.bytes("/dest/same (1).txt").unwrap(),
    ];
    bytes.sort();
    assert_eq!(bytes, [b"first".to_vec(), b"second".to_vec()]);
}
#[test]
fn strict_upload_discovery_skips_links_special_entries_and_preserves_empty_directories() {
    let fixture = Fixture::new();
    let folder = native("input/tree");
    fixture.local.seed_native_directory(&folder).unwrap();
    fixture
        .local
        .seed_native_directory(&folder.join("empty"))
        .unwrap();
    fixture
        .local
        .seed_native_file(&folder.join("file"), b"data".to_vec())
        .unwrap();
    fixture
        .local
        .seed_native_alias(&folder.join("cycle"), &folder)
        .unwrap();
    let progress = fixture.finished(fixture.upload(vec![folder], CollisionPolicy::Skip));
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.skipped_files, 1);
    assert_eq!(
        futures::executor::block_on(fixture.remote.stat("/dest/tree/empty"))
            .unwrap()
            .unwrap()
            .kind,
        nocterm_session::EntryKind::Directory
    );
}
#[test]
fn admission_rejects_oversized_sources_before_opening_any_writer() {
    let fixture = Fixture::new();
    let result = fixture.runtime.enqueue(UploadRequest {
        sources: vec![native("input/a"); 4097],
        target: nocterm_session::Target::new("u", "h", 22),
        destination: "/dest".into(),
        fs: fixture.remote.clone(),
        collisions: CollisionPolicy::Skip,
    });
    assert!(matches!(result, Err(TransferError::RequestTooLarge)));
    assert!(fixture.runtime.snapshot().is_empty());
    assert_eq!(fixture.remote.activity().live_uploads, 0);
    assert_eq!(fixture.remote.activity().requests, 1);
}
#[test]
fn shutdown_freezes_before_polling_and_retains_runtime_until_cleanup_finishes() {
    let fixture = Fixture::new();
    let source = native("input/file");
    fixture
        .local
        .seed_native_file(&source, b"data".to_vec())
        .unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Write, "/dest/file", Gate::pending());
    fixture.upload(vec![source.clone()], CollisionPolicy::Skip);
    wait(|| fixture.remote.activity().live_uploads == 1);
    let drain = fixture.runtime.freeze_and_drain();
    assert!(matches!(
        fixture.runtime.enqueue(UploadRequest {
            sources: vec![source],
            target: nocterm_session::Target::new("u", "h", 22),
            destination: "/dest".into(),
            fs: fixture.remote.clone(),
            collisions: CollisionPolicy::Skip
        }),
        Err(TransferError::ShuttingDown)
    ));
    let remote = fixture.remote.clone();
    drop(fixture);
    futures::executor::block_on(drain).unwrap();
    assert_eq!(remote.activity().live_uploads, 0);
    assert!(remote.bytes("/dest/file").is_err());
}

#[test]
fn snapshot_is_newest_first_and_live_retry_is_rejected_without_new_admission() {
    let fixture = Fixture::new();
    let source = native("input/file");
    fixture
        .local
        .seed_native_file(&source, b"data".to_vec())
        .unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Write, "/dest/file", Gate::pending());
    let first = fixture.upload(vec![source.clone()], CollisionPolicy::Skip);
    wait(|| fixture.remote.activity().live_uploads == 1);
    let second = fixture.upload(vec![source], CollisionPolicy::Skip);
    assert_eq!(
        fixture
            .runtime
            .snapshot()
            .iter()
            .map(|job| job.id)
            .collect::<Vec<_>>(),
        [second, first]
    );
    assert!(matches!(
        fixture.runtime.retry(first, fixture.remote.clone()),
        Err(TransferError::NotRetryable)
    ));
    assert_eq!(fixture.runtime.snapshot().len(), 2);
    futures::executor::block_on(fixture.runtime.freeze_and_drain()).unwrap();
    assert!(
        fixture
            .runtime
            .snapshot()
            .iter()
            .all(|job| job.state == TransferState::Cancelled)
    );
}
#[test]
fn finished_history_is_trimmed_after_completion_and_evicted_ids_are_typed_missing() {
    let limits = nocterm_transfers::TransferLimits::new(
        nocterm_transfers::QueueLimits::new(8, 32, 4, 2).unwrap(),
        nocterm_transfers::DiscoveryLimits::default(),
        nocterm_transfers::StreamLimits::default(),
    );
    let fixture = Fixture::with_limits(limits);
    let mut ids = Vec::new();
    for index in 0..5 {
        let source = native(&format!("input/file-{index}"));
        fixture
            .local
            .seed_native_file(&source, b"data".to_vec())
            .unwrap();
        let id = fixture.upload(vec![source], CollisionPolicy::Skip);
        fixture.finished(id);
        ids.push(id);
    }
    wait(|| fixture.runtime.snapshot().len() == 2);
    assert_eq!(
        fixture
            .runtime
            .snapshot()
            .iter()
            .map(|job| job.id)
            .collect::<Vec<_>>(),
        [ids[4], ids[3]]
    );
    assert!(matches!(
        fixture.runtime.cancel(ids[0]),
        Err(TransferError::Missing)
    ));
}
#[test]
fn visited_entry_exhaustion_stops_upload_cursor_traversal_instead_of_skipping_forever() {
    let limits = nocterm_transfers::TransferLimits::new(
        nocterm_transfers::QueueLimits::default(),
        nocterm_transfers::DiscoveryLimits::with_walk_bounds(2, 2, 100, 1024).unwrap(),
        nocterm_transfers::StreamLimits::default(),
    );
    let fixture = Fixture::with_limits(limits);
    let folder = native("input/tree");
    fixture.local.seed_native_directory(&folder).unwrap();
    for index in 0..10 {
        fixture
            .local
            .seed_native_file(&folder.join(format!("file-{index}")), b"data".to_vec())
            .unwrap();
    }
    let progress = fixture.finished(fixture.upload(vec![folder], CollisionPolicy::Skip));
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.failed_files, 1);
    assert!(matches!(
        progress.errors[0].cause(),
        nocterm_transfers::TransferCause::Limit(nocterm_transfers::LimitKind::Visited)
    ));
}
#[cfg(unix)]
#[test]
fn an_unrepresentable_native_name_is_reported_without_hiding_valid_siblings() {
    use std::os::unix::ffi::OsStringExt as _;
    let fixture = Fixture::new();
    let folder = native("input/tree");
    fixture.local.seed_native_directory(&folder).unwrap();
    fixture
        .local
        .seed_native_file(
            &folder.join(std::ffi::OsString::from_vec(vec![0xff])),
            b"bad".to_vec(),
        )
        .unwrap();
    fixture
        .local
        .seed_native_file(&folder.join("valid"), b"good".to_vec())
        .unwrap();
    let progress = fixture.finished(fixture.upload(vec![folder], CollisionPolicy::Skip));
    assert_eq!(progress.failed_files, 1);
    assert_eq!(progress.completed_files, 1);
    assert_eq!(fixture.remote.bytes("/dest/tree/valid").unwrap(), b"good");
}

#[test]
fn acknowledged_upload_directory_survives_later_source_listing_failure() {
    let fixture = Fixture::new();
    let source = native("input/tree");
    fixture.local.seed_native_directory(&source).unwrap();
    fixture.local.fail_next_local(
        FsOperation::List,
        source.clone(),
        nocterm_session::local_fs::LocalFsError::PermissionDenied(source.clone()),
    );
    let progress = fixture.finished(fixture.upload(vec![source], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.prepared_directories, 1);
    assert_eq!(progress.completed_files, 0);
    assert!(
        futures::executor::block_on(fixture.remote.stat("/dest/tree"))
            .unwrap()
            .is_some()
    );
}
#[test]
fn acknowledged_upload_directory_survives_cancellation_before_file_completion() {
    let fixture = Fixture::new();
    let source = native("input/tree");
    fixture.local.seed_native_directory(&source).unwrap();
    fixture
        .local
        .seed_native_file(&source.join("file"), b"data".to_vec())
        .unwrap();
    fixture
        .remote
        .suspend_next(FsOperation::Upload, "/dest/tree/file", Gate::pending());
    let id = fixture.upload(vec![source], CollisionPolicy::Skip);
    wait(|| {
        fixture
            .runtime
            .snapshot()
            .iter()
            .any(|p| p.id == id && p.prepared_directories == 1)
    });
    fixture.runtime.cancel(id).unwrap();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Cancelled);
    assert_eq!(progress.prepared_directories, 1);
    assert_eq!(progress.completed_files, 0);
    assert!(
        futures::executor::block_on(fixture.remote.stat("/dest/tree"))
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        fixture.remote.bytes("/dest/tree/file"),
        Err(FsError::NotFound { .. })
    ));
}
