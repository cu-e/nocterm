use super::*;
use crate::test_support::serve;

fn code() -> CountryCode {
    CountryCode::new("de").unwrap()
}
fn svg(length: usize) -> Vec<u8> {
    let mut bytes = b"<svg></svg>".to_vec();
    bytes.resize(length, b' ');
    bytes
}

#[test]
fn valid_disk_cache_precedes_http_even_at_exact_limit() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = svg(MAX_FLAG_BYTES as usize);
    std::fs::write(directory.path().join("de.svg"), &bytes).unwrap();
    let flags = CachedFlags::at(directory.path().into(), Some("http://127.0.0.1:0/".into()));
    assert_eq!(flags.load(code()).unwrap(), Some(bytes));
}

#[test]
fn oversized_or_invalid_cache_falls_through_to_network() {
    for cached in [svg(MAX_FLAG_BYTES as usize + 1), b"<html>".to_vec()] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("de.svg"), cached).unwrap();
        let bytes = svg(20);
        let (endpoint, worker) = serve(200, bytes.clone());
        assert_eq!(
            CachedFlags::at(directory.path().into(), Some(endpoint))
                .load(code())
                .unwrap(),
            Some(bytes.clone())
        );
        assert!(worker.join().unwrap().starts_with("GET /de.svg "));
        assert_eq!(
            std::fs::read(directory.path().join("de.svg")).unwrap(),
            bytes
        );
    }
}

#[test]
fn flag_download_bounds_and_svg_sniff_are_enforced() {
    for (bytes, accepted) in [
        (svg(MAX_FLAG_BYTES as usize), true),
        (svg(MAX_FLAG_BYTES as usize + 1), false),
        (b"<html>".to_vec(), false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (endpoint, worker) = serve(200, bytes.clone());
        let result = CachedFlags::at(directory.path().into(), Some(endpoint)).load(code());
        worker.join().unwrap();
        if accepted {
            assert_eq!(result.unwrap(), Some(bytes));
        } else {
            assert!(matches!(
                result,
                Err(ObservationError::BodyLimit { .. } | ObservationError::Decode(_))
            ));
        }
    }
}

#[test]
fn cache_publication_failure_retains_acquired_bytes_and_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("de.svg");
    // A directory cannot be replaced by an atomic file rename.
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep"), b"old").unwrap();
    let bytes = svg(20);
    let (endpoint, worker) = serve(200, bytes.clone());
    assert_eq!(
        CachedFlags::at(directory.path().into(), Some(endpoint))
            .load(code())
            .unwrap(),
        Some(bytes)
    );
    worker.join().unwrap();
    assert_eq!(std::fs::read(destination.join("keep")).unwrap(), b"old");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn offline_repository_uses_disk_and_never_downloads() {
    let directory = tempfile::tempdir().unwrap();
    let flags = CachedFlags::at(directory.path().into(), None);
    assert_eq!(flags.load(code()).unwrap(), None);
    std::fs::write(directory.path().join("de.svg"), b"<svg/>").unwrap();
    assert_eq!(flags.load(code()).unwrap(), Some(b"<svg/>".to_vec()));
}

#[test]
fn simultaneous_same_code_cache_publications_have_independent_stages() {
    let directory = tempfile::tempdir().unwrap();
    let first = svg(30);
    let second = svg(40);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let (one, worker_one) =
        crate::test_support::serve_after(200, first.clone(), Some(barrier.clone()));
    let (two, worker_two) = crate::test_support::serve_after(200, second.clone(), Some(barrier));
    std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            CachedFlags::at(directory.path().into(), Some(one))
                .load(code())
                .unwrap()
                .unwrap()
        });
        let two = scope.spawn(|| {
            CachedFlags::at(directory.path().into(), Some(two))
                .load(code())
                .unwrap()
                .unwrap()
        });
        let a = one.join().unwrap();
        let b = two.join().unwrap();
        assert!(a == first || a == second);
        assert!(b == first || b == second);
    });
    worker_one.join().unwrap();
    worker_two.join().unwrap();
    let saved = std::fs::read(directory.path().join("de.svg")).unwrap();
    assert!(saved == first || saved == second);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn failed_cache_staging_does_not_hide_downloaded_flag() {
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, b"keep").unwrap();
    let bytes = svg(20);
    let (endpoint, worker) = serve(200, bytes.clone());
    let result = CachedFlags::at(blocked.clone(), Some(endpoint))
        .load(code())
        .unwrap();
    worker.join().unwrap();
    assert_eq!(result, Some(bytes));
    assert_eq!(std::fs::read(&blocked).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn published_cache_is_private_and_replaces_exact_final_symlink() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other");
    std::fs::write(&other, b"<html>old</html>").unwrap();
    symlink(&other, directory.path().join("de.svg")).unwrap();
    let bytes = svg(20);
    let (endpoint, worker) = serve(200, bytes.clone());
    let result = CachedFlags::at(directory.path().into(), Some(endpoint))
        .load(code())
        .unwrap();
    worker.join().unwrap();
    assert_eq!(result, Some(bytes.clone()));
    let path = directory.path().join("de.svg");
    assert!(
        !std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(path.clone()).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read(other).unwrap(), b"<html>old</html>");
}
