use super::*;
use std::io::Write as _;

#[test]
fn binary_snapshot_round_trips_and_creates_parents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested/bytes");
    let bytes: Vec<_> = (0..524_288).map(|index| (index % 256) as u8).collect();
    write_private_atomic(&path, &bytes).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn discarded_snapshot_retains_old_bytes_and_cleans_staging() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot");
    fs::write(&path, b"old").unwrap();
    let staged = AtomicPublication::stage_private(&path, b"rejected revision").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"old");
    drop(staged);
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn failed_publication_preserves_destination_and_cleans_staging() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("existing"), b"preserve").unwrap();
    let staged = AtomicPublication::stage_private(&path, b"new").unwrap();
    assert!(staged.publish().is_err());
    assert_eq!(fs::read(path.join("existing")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn parent_creation_failure_leaves_existing_file_intact() {
    let directory = tempfile::tempdir().unwrap();
    let existing = directory.path().join("existing");
    fs::write(&existing, b"preserve").unwrap();
    assert!(write_private_atomic(&existing.join("child"), b"new").is_err());
    assert_eq!(fs::read(existing).unwrap(), b"preserve");
}

#[test]
fn exclusive_creation_refuses_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stream");
    let mut stream = create_private(&path).unwrap();
    stream.write_all(b"preserve").unwrap();
    assert_eq!(
        create_private(&path).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(path).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn exclusive_creation_refuses_regular_and_dangling_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let victim = directory.path().join("victim");
    fs::write(&victim, b"preserve").unwrap();
    let link = directory.path().join("stream");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    assert_eq!(
        create_private(&link).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&victim).unwrap(), b"preserve");
    fs::remove_file(&victim).unwrap();
    assert_eq!(
        create_private(&link).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!victim.exists());
}

#[cfg(unix)]
#[test]
fn private_publication_replaces_symlink_and_preserves_its_victim() {
    let directory = tempfile::tempdir().unwrap();
    let victim = directory.path().join("victim");
    let path = directory.path().join("snapshot");
    fs::write(&victim, b"preserve").unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    write_private_atomic(&path, b"snapshot").unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"preserve");
    assert_eq!(fs::read(&path).unwrap(), b"snapshot");
    assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

#[cfg(unix)]
#[test]
fn default_snapshots_and_streams_are_private_but_explicit_modes_are_retained() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let private = directory.path().join("private");
    fs::write(&private, b"old").unwrap();
    fs::set_permissions(&private, Permissions::from_mode(0o644)).unwrap();
    write_private_atomic(&private, b"private").unwrap();
    assert_eq!(
        fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let stream = directory.path().join("stream");
    create_private(&stream).unwrap();
    assert_eq!(
        fs::metadata(stream).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let shared = directory.path().join("shared");
    AtomicPublication::stage_with_permissions(
        &shared,
        b"trust record",
        Permissions::from_mode(0o644),
    )
    .unwrap()
    .publish()
    .unwrap();
    assert_eq!(
        fs::metadata(shared).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[derive(Debug, thiserror::Error)]
#[error("candidate rejected after prefix")]
struct RejectedSnapshot;

#[test]
fn streaming_producer_rejection_keeps_old_bytes_and_concrete_cause() {
    use std::error::Error as _;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot");
    fs::write(&path, b"old").unwrap();
    let result = AtomicPublication::stage_private_with(&path, |writer| {
        writer.write_all(b"partial candidate").unwrap();
        Err(RejectedSnapshot)
    });
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("rejected producer staged a candidate"),
    };
    assert!(matches!(error, StageError::Producer(_)));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<RejectedSnapshot>()
            .is_some()
    );
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn streaming_stage_is_discardable_and_publishes_exact_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot");
    fs::write(&path, b"old").unwrap();
    let stage = || {
        AtomicPublication::stage_private_with(&path, |writer| {
            writer.write_all(b"first")?;
            writer.write_all(b"second")
        })
        .unwrap()
    };
    drop(stage());
    assert_eq!(fs::read(&path).unwrap(), b"old");
    stage().publish().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"firstsecond");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn streaming_stage_native_failure_preserves_native_classification() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/snapshot");
    let result = AtomicPublication::stage_private_with(&path, |_| Ok::<_, RejectedSnapshot>(()));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("missing parent staged a candidate"),
    };
    assert!(
        matches!(error, StageError::Native(ref error) if error.kind() == io::ErrorKind::NotFound)
    );
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn streamed_private_snapshot_replaces_link_and_keeps_mode_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let victim = directory.path().join("victim");
    let path = directory.path().join("snapshot");
    fs::write(&victim, b"old victim").unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    AtomicPublication::stage_private_with(&path, |writer| writer.write_all(b"stream"))
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"old victim");
    assert_eq!(fs::read(&path).unwrap(), b"stream");
    assert!(
        !fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
