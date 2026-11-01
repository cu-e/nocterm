use super::*;
use nocterm_session::{
    FsError,
    testing::{FsOperation, Gate},
};
use nocterm_transfers::{
    CollisionPolicy, Request, TransferDirection, TransferError, TransferOperation,
    TransferRequests, TransferState,
};

#[test]
fn a_relayed_tree_arrives_byte_exact_with_bounded_pipelines() {
    let fixture = RelayFixture::new();
    for index in 0..40 {
        fixture
            .source
            .seed_file(
                &format!("/tree/dir/file-{index}"),
                vec![index as u8; 100_003],
            )
            .unwrap();
    }
    fixture.source.seed_file("/tree/top", b"top").unwrap();
    let id = fixture.relay(vec!["/tree"], CollisionPolicy::Skip);
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Completed);
    assert_eq!(progress.direction, TransferDirection::Relay);
    assert_eq!(progress.completed_files, 41);
    assert_eq!(progress.sent_bytes, 40 * 100_003 + 3);
    assert_eq!(progress.total_bytes, progress.sent_bytes);
    assert_eq!(
        fixture.destination.bytes("/dest/tree/dir/file-39").unwrap(),
        vec![39; 100_003]
    );
    assert_eq!(fixture.destination.bytes("/dest/tree/top").unwrap(), b"top");
    let source = fixture.source.activity();
    let destination = fixture.destination.activity();
    assert!(source.peak_downloads <= 4);
    assert!(source.max_read_request <= 32768);
    assert!(destination.peak_uploads <= 4);
    assert!(destination.max_write_chunk <= 32768);
    assert!(destination.max_write_batch <= 8);
    assert_eq!(destination.acknowledged_bytes as u64, progress.sent_bytes);
    assert_eq!(source.live_downloads, 0);
    assert_eq!(source.closed_downloads, 41);
    assert_eq!(destination.live_uploads, 0);
}

#[test]
fn relay_collision_choices_preserve_existing_bytes_and_rename_extensions() {
    for policy in [
        CollisionPolicy::Skip,
        CollisionPolicy::Rename,
        CollisionPolicy::Replace,
    ] {
        let fixture = RelayFixture::new();
        fixture
            .source
            .seed_file("/in/file.txt", b"incoming")
            .unwrap();
        fixture
            .destination
            .seed_file("/dest/file.txt", b"external")
            .unwrap();
        let progress = fixture.finished(fixture.relay(vec!["/in/file.txt"], policy));
        assert_eq!(progress.state, TransferState::Completed);
        match policy {
            CollisionPolicy::Skip => {
                assert_eq!(progress.skipped_files, 1);
                assert_eq!(
                    fixture.destination.bytes("/dest/file.txt").unwrap(),
                    b"external"
                );
            }
            CollisionPolicy::Rename => {
                assert_eq!(
                    fixture.destination.bytes("/dest/file.txt").unwrap(),
                    b"external"
                );
                assert_eq!(
                    fixture.destination.bytes("/dest/file (1).txt").unwrap(),
                    b"incoming"
                );
            }
            CollisionPolicy::Replace => assert_eq!(
                fixture.destination.bytes("/dest/file.txt").unwrap(),
                b"incoming"
            ),
        }
    }
}

#[test]
fn a_failed_source_read_never_publishes_and_releases_both_handles() {
    let fixture = RelayFixture::new();
    fixture
        .source
        .seed_file("/in/file", vec![1; 70_000])
        .unwrap();
    fixture
        .source
        .fail_next(FsOperation::Read, "/in/file", FsError::NotConnected);
    let progress = fixture.finished(fixture.relay(vec!["/in/file"], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.failed_files, 1);
    assert_eq!(progress.errors[0].operation(), TransferOperation::Read);
    assert!(fixture.destination.bytes("/dest/file").is_err());
    assert_eq!(fixture.source.activity().live_downloads, 0);
    assert_eq!(fixture.destination.activity().live_uploads, 0);
}

#[test]
fn a_failed_destination_write_never_publishes_and_closes_the_source() {
    let fixture = RelayFixture::new();
    fixture
        .source
        .seed_file("/in/file", vec![1; 70_000])
        .unwrap();
    fixture
        .destination
        .fail_next(FsOperation::Write, "/dest/file", FsError::NotConnected);
    let progress = fixture.finished(fixture.relay(vec!["/in/file"], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert_eq!(progress.errors[0].operation(), TransferOperation::Write);
    assert!(fixture.destination.bytes("/dest/file").is_err());
    assert_eq!(fixture.source.activity().live_downloads, 0);
    assert_eq!(fixture.destination.activity().live_uploads, 0);
}

#[test]
fn cancelling_a_stalled_relay_drains_without_publishing() {
    let fixture = RelayFixture::new();
    fixture
        .source
        .seed_file("/in/file", vec![1; 70_000])
        .unwrap();
    fixture
        .source
        .suspend_next(FsOperation::Read, "/in/file", Gate::pending());
    let id = fixture.relay(vec!["/in/file"], CollisionPolicy::Skip);
    wait(|| fixture.source.activity().live_downloads == 1);
    TransferRequests::cancel(fixture.runtime.as_ref(), id).unwrap();
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Cancelled);
    assert!(fixture.destination.bytes("/dest/file").is_err());
    assert_eq!(fixture.source.activity().live_downloads, 0);
    assert_eq!(fixture.destination.activity().live_uploads, 0);
}

#[test]
fn a_source_that_grows_past_its_declared_size_fails_the_relay() {
    let fixture = RelayFixture::new();
    fixture
        .source
        .seed_file("/in/file", vec![1; 40_000])
        .unwrap();
    fixture.source.declare_size("/in/file", Some(10)).unwrap();
    let progress = fixture.finished(fixture.relay(vec!["/in/file"], CollisionPolicy::Skip));
    assert_eq!(progress.state, TransferState::Failed);
    assert!(fixture.destination.bytes("/dest/file").is_err());
}

#[test]
fn a_remote_only_runtime_refuses_requests_that_need_local_files() {
    let fixture = RelayFixture::new();
    let upload = Request::Upload(nocterm_transfers::UploadRequest {
        sources: vec![native("input/file")],
        host: nocterm_session::HostAddress::direct(nocterm_session::Target::new("u", "h", 22)),
        destination: "/dest".into(),
        fs: fixture.destination.clone(),
        collisions: CollisionPolicy::Skip,
    });
    assert!(matches!(
        fixture.runtime.submit(upload),
        Err(TransferError::LocalUnavailable)
    ));
}

#[test]
fn a_missing_relay_source_fails_and_unknown_ids_are_missing() {
    let fixture = RelayFixture::new();
    let id = fixture.relay(vec!["/missing"], CollisionPolicy::Skip);
    let progress = fixture.finished(id);
    assert_eq!(progress.state, TransferState::Failed);
    let other = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    assert!(matches!(
        fixture.runtime.progress(other),
        Err(TransferError::Missing)
    ));
    assert!(matches!(
        TransferRequests::cancel(fixture.runtime.as_ref(), other),
        Err(TransferError::Missing)
    ));
}
