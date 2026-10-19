use super::*;
use futures::{executor::block_on, task::noop_waker};
use nocterm_session::testing::{FsOperation, Gate, InMemoryFs};
use std::task::{Context, Poll};
const RELEASE: &str = "/etc/os-release";
fn filesystem(bytes: &[u8]) -> Arc<InMemoryFs> {
    let fs = Arc::new(InMemoryFs::default());
    fs.seed_file(RELEASE, bytes).unwrap();
    fs
}
fn detected(fs: Arc<InMemoryFs>) -> Result<Option<&'static str>, OsObservationError> {
    block_on(NativeOsDetector.detect(fs)).map(|os| os.map(OsId::as_str))
}
#[test]
fn short_reads_close_and_use_platform_priority_only_for_eligible_bases() {
    for (release, marker, expected) in [
        ("ID=debian", "/usr/bin/pveversion", "proxmox"),
        ("ID=debian", "/etc/openmediavault", "openmediavault"),
        ("ID=ubuntu", "/usr/bin/pveversion", "ubuntu"),
        ("", "/usr/local/opnsense", "opnsense"),
        ("", "/bin/freebsd-version", "freebsd"),
    ] {
        let fs = filesystem(release.as_bytes());
        fs.set_limits(2, 32768).unwrap();
        fs.seed_file(marker, b"".to_vec()).unwrap();
        assert_eq!(detected(fs.clone()).unwrap(), Some(expected));
        assert_eq!(fs.activity().closed_downloads, 1);
        assert_eq!(fs.activity().live_downloads, 0);
        assert!(fs.activity().max_read_request <= 4096);
    }
}
#[test]
fn release_symlinks_and_special_files_fall_back_to_usr_release() {
    for symlink in [true, false] {
        let fs = Arc::new(InMemoryFs::default());
        if symlink {
            fs.seed_symlink(RELEASE, EntryKind::File).unwrap();
        } else {
            fs.seed_special(RELEASE).unwrap();
        }
        fs.seed_file("/usr/lib/os-release", b"ID=arch".to_vec())
            .unwrap();
        assert_eq!(detected(fs.clone()).unwrap(), Some("arch"));
        assert_eq!(fs.activity().closed_downloads, 1);
    }
}
#[test]
fn exact_limit_is_accepted_and_one_byte_over_closes_with_typed_failure() {
    for size in [MAX_OS_RELEASE_BYTES, MAX_OS_RELEASE_BYTES + 1] {
        let mut bytes = b"ID=debian\n#".to_vec();
        bytes.resize(size, b'x');
        let fs = filesystem(&bytes);
        let result = detected(fs.clone());
        if size == MAX_OS_RELEASE_BYTES {
            assert_eq!(result.unwrap(), Some("debian"));
        } else {
            assert!(matches!(
                result,
                Err(OsObservationError::Oversize {
                    limit: MAX_OS_RELEASE_BYTES,
                    ..
                })
            ));
        }
        assert_eq!(fs.activity().closed_downloads, 1);
        assert_eq!(fs.activity().live_downloads, 0);
    }
}
#[test]
fn hostile_chunk_and_invalid_utf8_close_before_returning_failure() {
    let fs = filesystem(b"ID=debian");
    fs.override_next_read(RELEASE, vec![0; 4097]).unwrap();
    assert!(matches!(
        detected(fs.clone()),
        Err(OsObservationError::InvalidChunk {
            requested: 4096,
            returned: 4097,
            ..
        })
    ));
    assert_eq!(fs.activity().closed_downloads, 1);
    let fs = filesystem(&[0xff]);
    assert!(matches!(
        detected(fs.clone()),
        Err(OsObservationError::InvalidUtf8 { .. })
    ));
    assert_eq!(fs.activity().closed_downloads, 1);
}
#[test]
fn read_failure_preserves_source_even_when_close_also_fails() {
    let fs = filesystem(b"ID=debian");
    fs.fail_next(FsOperation::Read, RELEASE, FsError::NotConnected);
    fs.fail_next(
        FsOperation::Close,
        RELEASE,
        FsError::Other("close refused".into()),
    );
    let error = detected(fs.clone()).unwrap_err();
    assert!(matches!(
        error,
        OsObservationError::Read {
            source: FsError::NotConnected,
            ..
        }
    ));
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<FsError>()
            .is_some()
    );
    assert_eq!(fs.activity().live_downloads, 0);
}
#[test]
fn successful_observation_survives_close_failure() {
    let fs = filesystem(b"ID=debian");
    fs.fail_next(FsOperation::Close, RELEASE, FsError::NotConnected);
    assert_eq!(detected(fs.clone()).unwrap(), Some("debian"));
    assert_eq!(fs.activity().live_downloads, 0);
}
#[test]
fn download_stat_home_and_unknown_errors_are_typed_and_not_silently_absent() {
    let fs = filesystem(b"ID=debian");
    fs.fail_next(
        FsOperation::Download,
        RELEASE,
        FsError::Other("unclassified native failure".into()),
    );
    assert!(matches!(
        detected(fs),
        Err(OsObservationError::Download {
            source: FsError::Other(_),
            ..
        })
    ));
    let fs = filesystem(b"ID=debian");
    fs.fail_next(
        FsOperation::Stat,
        RELEASE,
        FsError::PermissionDenied {
            path: RELEASE.into(),
        },
    );
    assert!(matches!(
        detected(fs),
        Err(OsObservationError::Stat {
            source: FsError::PermissionDenied { .. },
            ..
        })
    ));
    let fs = Arc::new(InMemoryFs::default());
    fs.fail_next(FsOperation::Home, "/", FsError::NotConnected);
    assert!(matches!(
        detected(fs),
        Err(OsObservationError::Home(FsError::NotConnected))
    ));
}
#[test]
fn successful_second_release_can_recover_from_invalid_first_release() {
    let fs = filesystem(&[0xff]);
    fs.seed_file(
        "/usr/lib/os-release",
        b"ID=fedora\nVARIANT_ID=coreos".to_vec(),
    )
    .unwrap();
    assert_eq!(detected(fs.clone()).unwrap(), Some("fedora-coreos"));
    assert_eq!(fs.activity().closed_downloads, 2);
}
#[test]
fn no_markers_is_absent_and_windows_home_is_recognized() {
    let fs = Arc::new(InMemoryFs::default());
    assert_eq!(detected(fs.clone()).unwrap(), None);
    block_on(fs.create_dir("/C:")).unwrap();
    block_on(fs.create_dir("/C:/Users")).unwrap();
    fs.set_home("/C:/Users").unwrap();
    assert_eq!(detected(fs).unwrap(), Some("windows"));
}
#[test]
fn canceled_read_releases_reader_without_claiming_async_close() {
    let fs = filesystem(b"ID=debian");
    fs.suspend_next(FsOperation::Read, RELEASE, Gate::pending());
    let mut future = NativeOsDetector.detect(fs.clone());
    let waker = noop_waker();
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    assert_eq!(fs.activity().live_downloads, 1);
    drop(future);
    assert_eq!(fs.activity().live_downloads, 0);
    assert_eq!(fs.activity().closed_downloads, 0);
}
#[test]
fn inert_detector_performs_zero_native_requests() {
    let fs = Arc::new(InMemoryFs::default());
    let detector = nocterm_connections_model::os_observation::OfflineOsDetector;
    assert!(block_on(detector.detect(fs.clone())).unwrap().is_none());
    assert_eq!(fs.activity().requests, 0);
}

#[test]
fn unsupported_metadata_can_use_stream_and_unsupported_download_can_fall_back() {
    let fs = filesystem(b"ID=debian");
    fs.fail_next(
        FsOperation::Stat,
        RELEASE,
        FsError::Unsupported("metadata".into()),
    );
    assert_eq!(detected(fs.clone()).unwrap(), Some("debian"));
    assert_eq!(fs.activity().closed_downloads, 1);
    let fs = filesystem(b"ID=debian");
    fs.fail_next(
        FsOperation::Download,
        RELEASE,
        FsError::Unsupported("download".into()),
    );
    fs.seed_file("/usr/lib/os-release", b"ID=arch".to_vec())
        .unwrap();
    assert_eq!(detected(fs.clone()).unwrap(), Some("arch"));
    assert_eq!(fs.activity().closed_downloads, 1);
}

#[test]
fn canceling_pending_close_releases_handle_without_an_acknowledged_close() {
    let fs = filesystem(b"ID=debian");
    fs.suspend_next(FsOperation::Close, RELEASE, Gate::pending());
    let mut future = NativeOsDetector.detect(fs.clone());
    let waker = noop_waker();
    assert!(matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    assert_eq!(fs.activity().live_downloads, 1);
    drop(future);
    assert_eq!(fs.activity().live_downloads, 0);
    assert_eq!(fs.activity().closed_downloads, 0);
}
