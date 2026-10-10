use crate::testing::{FsOperation, Gate, InMemoryFs};
use crate::{EntryKind, FsCapabilities, FsError, RemoteFs, fs::UploadMode};
use futures::{FutureExt as _, executor::block_on};

#[test]
fn snapshots_metadata_and_mutations_share_one_tree() {
    let fs = InMemoryFs::default();
    fs.seed_file("/home/me/one", b"abc").unwrap();
    fs.seed_symlink("/home/me/link", EntryKind::File).unwrap();
    block_on(async {
        assert_eq!(fs.home().await.unwrap(), "/");
        assert_eq!(fs.read_dir("/home/me").await.unwrap().len(), 2);
        assert_eq!(fs.metadata("/home/me/one").await.unwrap().size, Some(3));
        fs.set_permissions("/home/me/one", 0o640).await.unwrap();
        assert_eq!(
            fs.metadata("/home/me/one").await.unwrap().permissions,
            Some(0o640)
        );
        fs.rename("/home/me", "/home/new").await.unwrap();
        assert!(fs.stat("/home/me/one").await.unwrap().is_none());
        assert_eq!(fs.bytes("/home/new/one").unwrap(), b"abc");
        assert!(fs.remove_dir("/home/new").await.is_err());
        fs.remove_file("/home/new/link").await.unwrap();
        fs.remove_file("/home/new/one").await.unwrap();
        fs.remove_dir("/home/new").await.unwrap();
        assert!(fs.read_dir("/home").await.unwrap().is_empty());
    });
}

#[test]
fn links_special_files_and_invalid_paths_never_become_streams_or_ancestors() {
    let fs = InMemoryFs::default();
    fs.seed_symlink("/link", EntryKind::Directory).unwrap();
    fs.seed_special("/device").unwrap();
    block_on(async {
        assert!(fs.metadata("/link").await.unwrap().is_symlink);
        for path in [
            "/link",
            "/device",
            "/link/file",
            "relative",
            "/a/../f",
            "/a/./f",
            "/a//f",
        ] {
            assert!(fs.download(path).await.is_err(), "{path}");
            assert!(
                fs.upload(path, UploadMode::Replace).await.is_err(),
                "{path}"
            );
        }
        assert!(matches!(
            fs.set_permissions("/link", 0o600).await,
            Err(FsError::PermissionDenied { .. })
        ));
        assert!(matches!(
            fs.set_permissions("/device", 0o10000).await,
            Err(FsError::Unsupported(_))
        ));
        fs.remove_file("/link").await.unwrap();
        assert!(fs.stat("/link").await.unwrap().is_none());
    });
}

#[test]
fn staged_replace_is_invisible_until_finish_and_create_rechecks_late_collision() {
    let fs = InMemoryFs::default();
    fs.seed_file("/report", b"old").unwrap();
    block_on(async {
        let mut upload = fs.upload("/report", UploadMode::Replace).await.unwrap();
        upload.write(b"new".to_vec()).await.unwrap();
        assert_eq!(fs.bytes("/report").unwrap(), b"old");
        assert_eq!(upload.finish().await.unwrap(), "/report");
        assert_eq!(fs.bytes("/report").unwrap(), b"new");
        let mut collision = fs.upload("/late", UploadMode::Create).await.unwrap();
        collision.write(b"selected".to_vec()).await.unwrap();
        fs.seed_file("/late", b"external").unwrap();
        assert!(matches!(
            collision.finish().await,
            Err(FsError::AlreadyExists { .. })
        ));
        assert_eq!(fs.bytes("/late").unwrap(), b"external");
    });
    assert_eq!(fs.activity().completed_uploads, 1);
    assert_eq!(fs.activity().live_uploads, 0);
}

#[test]
fn failures_cancel_and_drop_discard_staging_without_touching_final_files() {
    let fs = InMemoryFs::default();
    fs.seed_file("/report", b"old").unwrap();
    block_on(async {
        let mut failed = fs.upload("/report", UploadMode::Replace).await.unwrap();
        failed.write(b"new".to_vec()).await.unwrap();
        fs.fail_next(
            FsOperation::Finish,
            "/report",
            FsError::PermissionDenied {
                path: "/report".into(),
            },
        );
        assert!(failed.finish().await.is_err());
        assert_eq!(fs.bytes("/report").unwrap(), b"old");
        let mut canceled = fs.upload("/cancel", UploadMode::Create).await.unwrap();
        canceled.write(b"discard".to_vec()).await.unwrap();
        canceled.cancel().await.unwrap();
        let abandoned = fs.upload("/drop", UploadMode::Create).await.unwrap();
        drop(abandoned);
        assert!(fs.stat("/cancel").await.unwrap().is_none());
        assert!(fs.stat("/drop").await.unwrap().is_none());
    });
    let activity = fs.activity();
    assert_eq!(activity.live_uploads, 0);
    assert_eq!(activity.canceled_uploads, 1);
    assert_eq!(activity.abandoned_uploads, 2);
}

#[test]
fn chunk_batch_and_file_limits_reject_before_mutating_staged_bytes() {
    let fs = InMemoryFs::default();
    fs.set_limits(3, 6).unwrap();
    block_on(async {
        let mut oversized = fs.upload("/file", UploadMode::Create).await.unwrap();
        assert!(oversized.write(vec![0; 4]).await.is_err());
        assert!(oversized.publish_as("file".into()).await.is_err());
        oversized.cancel().await.unwrap();
        let mut batch = fs.upload("/file", UploadMode::Create).await.unwrap();
        assert!(batch.write_batch(vec![vec![0]; 9]).await.is_err());
        assert!(batch.publish_as("file".into()).await.is_err());
        batch.cancel().await.unwrap();
        let mut full = fs.upload("/file", UploadMode::Create).await.unwrap();
        full.write_batch(vec![b"abc".to_vec(), b"def".to_vec()])
            .await
            .unwrap();
        assert!(full.write(vec![0]).await.is_err());
        assert!(full.finish().await.is_err());
        assert!(fs.bytes("/file").is_err());
        let mut valid = fs.upload("/file", UploadMode::Create).await.unwrap();
        valid
            .write_batch(vec![b"abc".to_vec(), b"def".to_vec()])
            .await
            .unwrap();
        valid.finish().await.unwrap();
    });
    assert_eq!(fs.bytes("/file").unwrap(), b"abcdef");
}

#[test]
fn bounded_reads_reach_eof_and_close_or_drop_releases_handles() {
    let fs = InMemoryFs::default();
    fs.set_limits(3, 20).unwrap();
    fs.seed_file("/file", b"abcdefg").unwrap();
    block_on(async {
        let mut reader = fs.download("/file").await.unwrap();
        assert_eq!(reader.read(20).await.unwrap(), b"abc");
        assert_eq!(reader.read(2).await.unwrap(), b"de");
        assert_eq!(reader.read(20).await.unwrap(), b"fg");
        assert!(reader.read(20).await.unwrap().is_empty());
        reader.close().await.unwrap();
        let dropped = fs.download("/file").await.unwrap();
        drop(dropped);
    });
    let activity = fs.activity();
    assert_eq!(activity.closed_downloads, 1);
    assert_eq!(activity.live_downloads, 0);
    assert_eq!(activity.max_read_request, 20);
}

#[test]
fn suspended_and_failed_reads_do_not_skip_bytes_when_the_future_is_canceled() {
    let fs = InMemoryFs::default();
    fs.seed_file("/file", b"abcdef").unwrap();
    block_on(async {
        let mut reader = fs.download("/file").await.unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Read, "/file", gate.clone());
        let mut pending = reader.read(2);
        assert!(pending.as_mut().now_or_never().is_none());
        drop(pending);
        gate.release();
        fs.fail_next(FsOperation::Read, "/file", FsError::NotConnected);
        assert_eq!(reader.read(2).await, Err(FsError::NotConnected));
        assert_eq!(reader.read(2).await.unwrap(), b"ab");
        reader.close().await.unwrap();
    });
}

#[test]
fn shutdown_and_capability_failures_are_explicit_and_pending_finish_preserves_old_bytes() {
    let fs = InMemoryFs::default();
    fs.seed_file("/report", b"old").unwrap();
    block_on(async {
        let mut upload = fs.upload("/report", UploadMode::Replace).await.unwrap();
        upload.write(b"new".to_vec()).await.unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Finish, "/report", gate.clone());
        let mut pending = upload.finish();
        assert!(pending.as_mut().now_or_never().is_none());
        assert_eq!(fs.bytes("/report").unwrap(), b"old");
        gate.release();
        pending.await.unwrap();
        fs.set_capabilities(FsCapabilities::default());
        assert!(matches!(
            fs.rename("/report", "/new").await,
            Err(FsError::Unsupported(_))
        ));
        assert!(matches!(
            fs.metadata("/report").await,
            Err(FsError::Unsupported(_))
        ));
    });
}

#[test]
fn competing_staged_creates_publish_exactly_one_complete_file() {
    let fs = InMemoryFs::default();
    block_on(async {
        let mut first = fs.upload("/same", UploadMode::Create).await.unwrap();
        let mut second = fs.upload("/same", UploadMode::Create).await.unwrap();
        first.write(b"first".to_vec()).await.unwrap();
        second.write(b"second".to_vec()).await.unwrap();
        let (first, second) = futures::join!(first.finish(), second.finish());
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let bytes = fs.bytes("/same").unwrap();
        assert!(bytes == b"first" || bytes == b"second");
    });
    assert_eq!(fs.activity().live_uploads, 0);
    assert_eq!(fs.activity().completed_uploads, 1);
}

#[test]
fn canceled_pending_write_does_not_change_staging_and_failed_close_releases_reader() {
    let fs = InMemoryFs::default();
    block_on(async {
        let mut upload = fs.upload("/file", UploadMode::Create).await.unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Write, "/file", gate.clone());
        let mut pending = upload.write(b"discarded".to_vec());
        assert!(pending.as_mut().now_or_never().is_none());
        drop(pending);
        gate.release();
        assert!(
            upload.publish_as("file".into()).await.is_err(),
            "An unacknowledged stage cannot authorize publication"
        );
        upload.cancel().await.unwrap();
        assert!(fs.bytes("/file").is_err());
        let mut upload = fs.upload("/file", UploadMode::Create).await.unwrap();
        upload.write(b"kept".to_vec()).await.unwrap();
        upload.finish().await.unwrap();
        assert_eq!(fs.bytes("/file").unwrap(), b"kept");
        let mut reader = fs.download("/file").await.unwrap();
        assert!(reader.read(0).await.is_err());
        assert_eq!(reader.read(100).await.unwrap(), b"kept");
        fs.fail_next(FsOperation::Close, "/file", FsError::NotConnected);
        assert_eq!(reader.close().await, Err(FsError::NotConnected));
    });
    assert_eq!(fs.activity().live_downloads, 0);
    assert_eq!(fs.activity().closed_downloads, 0);
}

#[test]
fn scripted_listings_preserve_request_order_and_report_canceled_consumers() {
    use crate::{DirEntry, testing::ListingReplyError};
    let fs = InMemoryFs::default();
    fs.seed_file("/tree/file", b"kept").unwrap();
    assert_eq!(block_on(fs.read_dir("/tree")).unwrap().len(), 1);
    fs.script_listings();
    let mut first = fs.read_dir("/first");
    let mut second = fs.read_dir("/second");
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    let reply = fs.take_pending_listing().unwrap();
    assert_eq!(reply.path(), "/first");
    // Deliberately invalid names reach the consumer's protocol validator.
    reply
        .send(Ok(vec![DirEntry {
            name: "../escape".into(),
            kind: EntryKind::Directory,
            is_symlink: false,
            size: None,
        }]))
        .unwrap();
    assert_eq!(block_on(first).unwrap()[0].name, "../escape");
    let canceled = fs.take_pending_listing().unwrap();
    assert_eq!(canceled.path(), "/second");
    drop(second);
    assert_eq!(canceled.send(Ok(vec![])), Err(ListingReplyError::Closed));
    assert!(!fs.has_pending_listings());
    assert_eq!(fs.bytes("/tree/file").unwrap(), b"kept");
}

#[test]
fn scripted_listing_responses_and_pending_requests_have_independent_bounds() {
    use crate::{DirEntry, testing::ListingReplyError};
    let fs = InMemoryFs::default();
    fs.script_listings();
    let mut pending = Vec::new();
    for _ in 0..64 {
        let mut request = fs.read_dir("/pending");
        assert!(request.as_mut().now_or_never().is_none());
        pending.push(request);
    }
    assert!(matches!(
        block_on(fs.read_dir("/excess")),
        Err(FsError::Unsupported(_))
    ));
    let reply = fs.take_pending_listing().unwrap();
    assert_eq!(
        reply.send(Ok(vec![DirEntry {
            name: "x".repeat(16 * 1024 * 1024 + 1),
            kind: EntryKind::File,
            is_symlink: false,
            size: None,
        }])),
        Err(ListingReplyError::TooLarge)
    );
    assert!(matches!(
        block_on(pending.remove(0)),
        Err(FsError::NotConnected)
    ));
}

#[test]
fn late_publication_collision_retains_the_same_sealed_bytes_for_a_leaf_retry() {
    let fs = InMemoryFs::default();
    block_on(async {
        let mut upload = fs.upload("/result.txt", UploadMode::Create).await.unwrap();
        upload
            .write_batch(vec![b"incoming".to_vec()])
            .await
            .unwrap();
        fs.seed_file("/result.txt", b"external").unwrap();
        assert!(matches!(
            upload.publish_as("result.txt".into()).await,
            Err(FsError::AlreadyExists { .. })
        ));
        assert_eq!(fs.bytes("/result.txt").unwrap(), b"external");
        assert!(upload.write(b"extra".to_vec()).await.is_err());
        assert_eq!(
            upload.publish_as("result (1).txt".into()).await.unwrap(),
            "/result (1).txt"
        );
        assert_eq!(fs.bytes("/result (1).txt").unwrap(), b"incoming");
        assert!(upload.publish_as("another".into()).await.is_err());
        assert_eq!(fs.activity().completed_uploads, 1);
    });
    assert_eq!(fs.activity().live_uploads, 0);
}
#[test]
fn publication_leaf_validation_and_failed_ack_never_publish_or_escape_the_parent() {
    let fs = InMemoryFs::default();
    block_on(async {
        fs.create_dir("/parent").await.unwrap();
        let mut upload = fs.upload("/parent/file", UploadMode::Create).await.unwrap();
        upload.write(b"private".to_vec()).await.unwrap();
        for name in ["", ".", "..", "../outside", "/outside", "a\0b"] {
            assert!(upload.publish_as(name.into()).await.is_err());
        }
        assert!(upload.publish_as("a".repeat(1025)).await.is_err());
        assert!(fs.bytes("/outside").is_err());
        fs.fail_next(FsOperation::Write, "/parent/file", FsError::NotConnected);
        assert!(upload.write(b"uncertain".to_vec()).await.is_err());
        assert!(upload.publish_as("file".into()).await.is_err());
        upload.cancel().await.unwrap();
        assert!(fs.bytes("/parent/file").is_err());
    });
}

#[test]
fn canceled_pending_publication_seals_and_poisoned_stage_cannot_publish_again() {
    let fs = InMemoryFs::default();
    block_on(async {
        let mut upload = fs.upload("/file", UploadMode::Create).await.unwrap();
        upload.write(b"private".to_vec()).await.unwrap();
        let gate = Gate::pending();
        fs.suspend_next(FsOperation::Finish, "/file", gate.clone());
        let mut publication = upload.publish_as("file".into());
        assert!(publication.as_mut().now_or_never().is_none());
        assert!(upload.write(b"late".to_vec()).await.is_err());
        drop(publication);
        gate.release();
        assert!(upload.publish_as("other".into()).await.is_err());
        assert!(fs.bytes("/file").is_err());
        assert!(fs.bytes("/other").is_err());
        upload.cancel().await.unwrap();
    });
    assert_eq!(fs.activity().live_uploads, 0);
    assert_eq!(fs.activity().completed_uploads, 0);
}
