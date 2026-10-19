use super::*;
use nocterm_session::{
    EntryKind, FsError, RemoteFs,
    local_fs::{LocalFs, LocalFsError, LocalMutations},
    testing::{FsOperation, Gate},
};
use nocterm_transfers::{
    CollisionPolicy, DiscoveryLimits, DownloadRequest, QueueLimits, StreamLimits, TransferCause,
    TransferError, TransferLimits, TransferService, TransferState,
};
use std::ffi::OsStr;

#[test]
fn many_files_stream_with_bounded_readers_and_exact_accepted_bytes() {
    let fixture = Fixture::new();
    let mut sources = Vec::new();
    for index in 0..100 {
        let source = format!("/source/file-{index}");
        fixture
            .remote
            .seed_file(&source, vec![index as u8; 65539])
            .unwrap();
        sources.push(source);
    }
    let progress = fixture.finished(fixture.download(sources, CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.completed_files, 100);
    assert_eq!(progress.sent_bytes, 100 * 65539);
    assert_eq!(fixture.local_bytes("file-99"), vec![99; 65539]);
    let activity = fixture.remote.activity();
    assert!(activity.peak_downloads <= 4);
    assert!(activity.max_read_request <= 32768);
    assert_eq!(activity.live_downloads, 0);
    assert_eq!(activity.closed_downloads, 100);
}
#[test]
fn local_publication_is_invisible_until_complete_and_collision_choices_keep_old_bytes() {
    for policy in [
        CollisionPolicy::Skip,
        CollisionPolicy::Rename,
        CollisionPolicy::Replace,
    ] {
        let fixture = Fixture::new();
        fixture.remote.seed_file("/file.txt", b"incoming").unwrap();
        fixture
            .local
            .seed_native_file(&native("output/file.txt"), b"external".to_vec())
            .unwrap();
        let gate = Gate::pending();
        fixture
            .remote
            .suspend_next(FsOperation::Read, "/file.txt", gate.clone());
        let id = fixture.download(vec!["/file.txt".into()], policy);
        if policy != CollisionPolicy::Skip {
            wait(|| fixture.remote.activity().live_downloads == 1);
        }
        assert_eq!(fixture.local_bytes("file.txt"), b"external");
        gate.release();
        let progress = fixture.finished(id);
        assert_eq!(progress.state, TransferState::Completed);
        match policy {
            CollisionPolicy::Skip => {
                assert_eq!(progress.skipped_files, 1);
                assert_eq!(fixture.local_bytes("file.txt"), b"external");
            }
            CollisionPolicy::Rename => {
                assert_eq!(fixture.local_bytes("file.txt"), b"external");
                assert_eq!(fixture.local_bytes("file (1).txt"), b"incoming");
            }
            CollisionPolicy::Replace => assert_eq!(fixture.local_bytes("file.txt"), b"incoming"),
        }
    }
}
#[test]
fn late_local_collision_renames_the_same_synced_stage_without_reopening_the_reader() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/file.txt", b"incoming").unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Close, "/file.txt", gate.clone());
    let id = fixture.download(vec!["/file.txt".into()], CollisionPolicy::Rename);
    wait(|| {
        fixture
            .runtime
            .snapshot()
            .iter()
            .any(|job| job.id == id && job.sent_bytes == 8)
    });
    fixture
        .local
        .seed_native_file(&native("output/file.txt"), b"external".to_vec())
        .unwrap();
    fixture
        .remote
        .seed_file("/file.txt", b"must-not-be-read-again")
        .unwrap();
    gate.release();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(fixture.local_bytes("file.txt"), b"external");
    assert_eq!(fixture.local_bytes("file (1).txt"), b"incoming");
    assert_eq!(fixture.remote.activity().closed_downloads, 1);
}
#[test]
fn cancelled_or_failed_reads_discard_private_output_and_release_both_handles() {
    for cancel in [true, false] {
        let fixture = Fixture::new();
        fixture.remote.seed_file("/file", vec![1; 65536]).unwrap();
        if cancel {
            fixture
                .remote
                .suspend_next(FsOperation::Read, "/file", Gate::pending());
        } else {
            fixture
                .remote
                .fail_next(FsOperation::Read, "/file", FsError::NotConnected);
        }
        let id = fixture.download(vec!["/file".into()], CollisionPolicy::Skip);
        if cancel {
            wait(|| fixture.remote.activity().live_downloads == 1);
            fixture.runtime.cancel(id).unwrap();
        }
        let progress = fixture.finished(id);
        assert_eq!(
            progress.state,
            if cancel {
                TransferState::Cancelled
            } else {
                TransferState::Failed
            }
        );
        assert_eq!(progress.sent_bytes, 0);
        assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/file")).is_err());
        assert_eq!(fixture.remote.activity().live_downloads, 0);
        assert_eq!(fixture.local.activity().live_uploads, 0);
    }
}
#[test]
fn oversized_remote_chunk_is_typed_and_cannot_publish_a_local_file() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/file", b"incoming").unwrap();
    fixture
        .remote
        .override_next_read("/file", vec![1; 32769])
        .unwrap();
    let progress = fixture.finished(fixture.download(vec!["/file".into()], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert!(matches!(
        progress.errors[0].cause(),
        TransferCause::OversizedChunk { .. }
    ));
    assert_eq!(progress.sent_bytes, 0);
    assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/file")).is_err());
}
#[test]
fn declared_short_growth_and_unknown_size_sources_have_distinct_publication_results() {
    for declared in [Some(2), Some(20), None] {
        let fixture = Fixture::new();
        fixture.remote.seed_file("/file", b"incoming").unwrap();
        fixture.remote.declare_size("/file", declared).unwrap();
        let progress =
            fixture.finished(fixture.download(vec!["/file".into()], CollisionPolicy::Skip));
        if declared.is_none() {
            assert_eq!(progress.state, TransferState::Completed);
            assert_eq!(fixture.local_bytes("file"), b"incoming");
        } else {
            assert_eq!(progress.state, TransferState::Failed);
            assert!(matches!(
                progress.errors[0].cause(),
                TransferCause::SizeChanged { .. }
            ));
            assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/file")).is_err());
        }
    }
}
#[test]
fn destination_links_are_refused_and_held_parent_replacement_cannot_redirect_output() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/file", b"incoming").unwrap();
    fixture
        .local
        .seed_native_directory(&native("outside"))
        .unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Read, "/file", gate.clone());
    let id = fixture.download(vec!["/file".into()], CollisionPolicy::Skip);
    wait(|| fixture.remote.activity().live_downloads == 1);
    LocalMutations::rename(
        fixture.local.as_ref(),
        &native("output"),
        OsStr::new("moved"),
    )
    .unwrap();
    fixture
        .local
        .seed_native_alias(&native("output"), &native("outside"))
        .unwrap();
    gate.release();
    assert_eq!(fixture.finished(id).state, TransferState::Completed);
    assert!(LocalFs::open_source(fixture.local.as_ref(), &native("outside/file")).is_err());
    let mut source = LocalFs::open_source(fixture.local.as_ref(), &native("moved/file")).unwrap();
    assert_eq!(source.read(32).unwrap(), b"incoming");
    source.close().unwrap();
    let next = fixture.download(vec!["/file".into()], CollisionPolicy::Skip);
    assert_eq!(fixture.finished(next).state, TransferState::Failed);
}
#[test]
fn aliased_held_destination_identity_serializes_same_basename_replace() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/a/same", b"first").unwrap();
    fixture.remote.seed_file("/b/same", b"second").unwrap();
    let progress = fixture.finished(fixture.download(
        vec!["/a/same".into(), "/b/same".into()],
        CollisionPolicy::Replace,
    ));
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.completed_files, 2);
    let bytes = fixture.local_bytes("same");
    assert!(bytes == b"first" || bytes == b"second");
    assert_eq!(fixture.local.activity().live_uploads, 0);
}
#[test]
fn empty_directories_links_special_files_and_hostile_names_are_accounted_without_escape() {
    let fixture = Fixture::new();
    futures::executor::block_on(fixture.remote.create_dir("/tree")).unwrap();
    futures::executor::block_on(fixture.remote.create_dir("/tree/empty")).unwrap();
    fixture.remote.seed_file("/tree/file", b"data").unwrap();
    fixture
        .remote
        .seed_symlink("/tree/link", EntryKind::Directory)
        .unwrap();
    fixture.remote.seed_special("/tree/socket").unwrap();
    fixture.remote.seed_file("/tree/CON", b"bad").unwrap();
    let progress = fixture.finished(fixture.download(vec!["/tree".into()], CollisionPolicy::Skip));
    assert_eq!(progress.completed_files, 1);
    assert_eq!(progress.skipped_files, 2);
    assert_eq!(progress.failed_files, 1);
    assert_eq!(fixture.local_bytes("tree/file"), b"data");
    assert!(LocalFs::open_directory(fixture.local.as_ref(), &native("output/tree/empty")).is_ok());
    assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/tree/CON")).is_err());
}
#[test]
fn hostile_sources_and_oversized_requests_fail_before_reader_or_stage_admission() {
    let fixture = Fixture::new();
    for sources in [vec!["/../escape".into()], vec!["/a".into(); 4097]] {
        let result = fixture.runtime.enqueue_download(DownloadRequest {
            sources,
            target: nocterm_session::Target::new("u", "h", 22),
            local_destination: native("output"),
            fs: fixture.remote.clone(),
            collisions: CollisionPolicy::Skip,
        });
        assert!(matches!(
            result,
            Err(TransferError::InvalidDestination) | Err(TransferError::RequestTooLarge)
        ));
    }
    assert!(fixture.runtime.snapshot().is_empty());
    assert_eq!(fixture.remote.activity().live_downloads, 0);
    assert_eq!(fixture.local.activity().live_uploads, 0);
}
#[test]
fn small_validated_depth_and_pending_listing_limits_fail_without_unbounded_recursion() {
    let limits = TransferLimits::new(
        QueueLimits::default(),
        DiscoveryLimits::with_walk_bounds(2, 10, 2, 16).unwrap(),
        StreamLimits::default(),
    );
    let fixture = Fixture::with_limits(limits);
    fixture.remote.seed_file("/tree/a/b/file", b"data").unwrap();
    let progress = fixture.finished(fixture.download(vec!["/tree".into()], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert!(
        progress
            .errors
            .iter()
            .any(|issue| matches!(issue.cause(), TransferCause::Limit(_)))
    );
    let fixture = Fixture::with_limits(limits);
    for name in ["a", "b", "c"] {
        fixture
            .remote
            .seed_file(&format!("/tree/{name}"), b"data")
            .unwrap();
    }
    let progress = fixture.finished(fixture.download(vec!["/tree".into()], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.completed_files, 0);
}
#[test]
fn committed_directory_durability_warning_is_completed_and_not_retryable_failure() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/file", b"incoming").unwrap();
    fixture.local.fail_next_local(
        FsOperation::DirectorySync,
        native("output"),
        nocterm_session::local_fs::LocalFsError::from_io(
            native("output"),
            std::io::Error::other("flush"),
        ),
    );
    let progress = fixture.finished(fixture.download(vec!["/file".into()], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.failed_files, 0);
    assert_eq!(progress.completed_files, 1);
    assert!(matches!(
        progress.errors[0].cause(),
        TransferCause::Durability(_)
    ));
    assert_eq!(fixture.local_bytes("file"), b"incoming");
}

#[test]
fn late_skip_collision_discards_the_stage_and_keeps_external_creator_bytes() {
    let fixture = Fixture::new();
    fixture.remote.seed_file("/file", b"incoming").unwrap();
    let gate = Gate::pending();
    fixture
        .remote
        .suspend_next(FsOperation::Close, "/file", gate.clone());
    let id = fixture.download(vec!["/file".into()], CollisionPolicy::Skip);
    wait(|| {
        fixture
            .runtime
            .snapshot()
            .iter()
            .any(|job| job.id == id && job.sent_bytes == 8)
    });
    fixture
        .local
        .seed_native_file(&native("output/file"), b"external".to_vec())
        .unwrap();
    gate.release();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.skipped_files, 1);
    assert_eq!(progress.completed_files, 0);
    assert_eq!(fixture.local_bytes("file"), b"external");
    assert_eq!(fixture.local.activity().live_uploads, 0);
}
#[test]
fn failed_reader_close_or_local_sync_never_authorizes_publication() {
    for failed_close in [true, false] {
        let fixture = Fixture::new();
        fixture.remote.seed_file("/file", b"incoming").unwrap();
        if failed_close {
            fixture
                .remote
                .fail_next(FsOperation::Close, "/file", FsError::NotConnected);
        } else {
            fixture.local.fail_next_local(
                FsOperation::Sync,
                native("output"),
                nocterm_session::local_fs::LocalFsError::from_io(
                    native("output"),
                    std::io::Error::other("sync"),
                ),
            );
        }
        let progress =
            fixture.finished(fixture.download(vec!["/file".into()], CollisionPolicy::Skip));
        assert_eq!(progress.state, TransferState::Failed);
        assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/file")).is_err());
        assert_eq!(fixture.local.activity().live_uploads, 0);
        assert_eq!(fixture.remote.activity().live_downloads, 0);
    }
}

#[test]
fn exhausted_download_walk_stops_instead_of_reporting_every_remaining_name() {
    let discovery = nocterm_transfers::DiscoveryLimits::with_walk_bounds(4, 2, 100, 4096).unwrap();
    let limits = TransferLimits::new(Default::default(), discovery, Default::default());
    let fixture = Fixture::with_limits(limits);
    futures::executor::block_on(fixture.remote.create_dir("/dest/tree")).unwrap();
    for index in 0..50 {
        fixture
            .remote
            .seed_file(&format!("/dest/tree/{index}.txt"), b"x".to_vec())
            .unwrap();
    }
    let id = fixture.download(vec!["/dest/tree".into()], CollisionPolicy::Rename);
    let progress = fixture.finished(id);
    assert_eq!(progress.discovered_files, 1);
    assert_eq!(progress.completed_files, 1);
    assert_eq!(
        progress.failed_files, 1,
        "one terminal walk-limit diagnostic"
    );
    assert!(progress.errors.iter().any(|error| matches!(
        error.cause(),
        nocterm_transfers::TransferCause::Limit(nocterm_transfers::LimitKind::Visited)
    )));
}

#[test]
fn acknowledged_download_directory_survives_failed_or_cancelled_remote_listing() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        futures::executor::block_on(fixture.remote.create_dir("/tree")).unwrap();
        if cancelled {
            fixture
                .remote
                .suspend_next(FsOperation::List, "/tree", Gate::pending());
        } else {
            fixture.remote.fail_next(
                FsOperation::List,
                "/tree",
                FsError::PermissionDenied {
                    path: "/tree".into(),
                },
            );
        }
        let id = fixture.download(vec!["/tree".into()], CollisionPolicy::Skip);
        if cancelled {
            wait(|| {
                fixture
                    .runtime
                    .snapshot()
                    .iter()
                    .any(|p| p.id == id && p.prepared_directories == 1)
            });
            fixture.runtime.cancel(id).unwrap();
        }
        let progress = fixture.finished(id);
        assert_eq!(
            progress.state,
            if cancelled {
                TransferState::Cancelled
            } else {
                TransferState::Failed
            }
        );
        assert_eq!(progress.prepared_directories, 1);
        assert_eq!(progress.completed_files, 0);
        assert!(LocalFs::open_directory(fixture.local.as_ref(), &native("output/tree")).is_ok());
    }
}
#[test]
fn rejected_destination_directory_cannot_report_a_preparation() {
    let fixture = Fixture::new();
    futures::executor::block_on(fixture.remote.create_dir("/tree")).unwrap();
    fixture.local.fail_next_local(
        FsOperation::CreateDir,
        native("output/tree"),
        nocterm_session::local_fs::LocalFsError::PermissionDenied(native("output/tree")),
    );
    let progress = fixture.finished(fixture.download(vec!["/tree".into()], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.prepared_directories, 0);
    assert_eq!(progress.completed_files, 0);
    assert!(LocalFs::open_directory(fixture.local.as_ref(), &native("output/tree")).is_err());
}

#[test]
fn failed_and_cancelled_downloads_report_explicit_stage_cleanup_failure() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        fixture.remote.seed_file("/file", b"private").unwrap();
        fixture.local.fail_next_local(
            FsOperation::Cancel,
            native("output"),
            LocalFsError::PermissionDenied(native("output")),
        );
        if cancelled {
            fixture
                .remote
                .suspend_next(FsOperation::Read, "/file", Gate::pending());
        } else {
            fixture
                .remote
                .fail_next(FsOperation::Read, "/file", FsError::NotConnected);
        }
        let id = fixture.download(vec!["/file".into()], CollisionPolicy::Skip);
        if cancelled {
            wait(|| fixture.local.activity().live_uploads == 1);
            fixture.runtime.cancel(id).unwrap();
        }
        let progress = fixture.finished(id);
        assert_eq!(
            progress.state,
            if cancelled {
                TransferState::Cancelled
            } else {
                TransferState::Failed
            }
        );
        assert_eq!(progress.completed_files, 0);
        assert!(progress.errors.iter().any(|notice|
            notice.operation() == nocterm_transfers::TransferOperation::Cleanup
                && matches!(notice.cause(), TransferCause::Local(LocalFsError::PermissionDenied(path)) if path == &native("output"))));
        assert!(LocalFs::open_source(fixture.local.as_ref(), &native("output/file")).is_err());
        assert_eq!(fixture.local.activity().live_uploads, 0);
        assert_eq!(fixture.remote.activity().live_downloads, 0);
    }
}
