use super::*;

#[test]
fn namespace_commit_is_visible_before_parent_sync_and_receipt_drop_never_rolls_it_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot");
    fs::write(&path, b"old").unwrap();
    let committed = AtomicPublication::stage_private(&path, b"new")
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new");
    drop(committed);
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    AtomicPublication::stage_private(&path, b"synced")
        .unwrap()
        .commit()
        .unwrap()
        .sync_parent();
    assert_eq!(fs::read(path).unwrap(), b"synced");
}

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
fn new_publication_has_one_winner_and_never_overwrites_an_existing_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity");
    let first = AtomicPublication::stage_private(&path, b"first identity").unwrap();
    let second = AtomicPublication::stage_private(&path, b"second identity").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let ready = barrier.clone();
    let one = std::thread::spawn(move || {
        ready.wait();
        first.publish_new()
    });
    let two = std::thread::spawn(move || {
        barrier.wait();
        second.publish_new()
    });
    let outcomes = [one.join().unwrap(), two.join().unwrap()];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter_map(|result| result.as_ref().err())
            .filter(|error| error.kind() == io::ErrorKind::AlreadyExists)
            .count(),
        1
    );
    let bytes = fs::read(&path).unwrap();
    assert!(bytes == b"first identity" || bytes == b"second identity");
    assert!(
        AtomicPublication::stage_private(&path, b"replacement")
            .unwrap()
            .publish_new()
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn new_publication_preserves_live_and_dangling_links_without_reading_their_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let path = directory.path().join("identity");
    fs::write(&target, b"preserve").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    for exists in [true, false] {
        if !exists {
            fs::remove_file(&target).unwrap();
        }
        let result = AtomicPublication::stage_private(&path, b"new identity")
            .unwrap()
            .publish_new();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_link(&path).unwrap(), target);
        if exists {
            assert_eq!(fs::read(&target).unwrap(), b"preserve");
        } else {
            assert!(!target.exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn private_open_checks_actual_permissions_and_links_and_retains_original_file() {
    use std::{io::Read as _, os::unix::fs::PermissionsExt as _};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity");
    let published = AtomicPublication::stage_private(&path, b"original")
        .unwrap()
        .publish_new()
        .unwrap();
    require_private_regular(&published).unwrap();
    for mode in [0o600, 0o400] {
        fs::set_permissions(&path, Permissions::from_mode(mode)).unwrap();
        let mut original = open_private_regular(&path).unwrap();
        let mut bytes = Vec::new();
        original.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original");
    }
    for mode in [0o640, 0o604, 0o700, 0o4600] {
        fs::set_permissions(&path, Permissions::from_mode(mode)).unwrap();
        assert!(matches!(
            open_private_regular(&path),
            Err(PrivateFileError::Policy)
        ));
    }
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    let link = directory.path().join("backup-link");
    fs::hard_link(&path, &link).unwrap();
    assert!(matches!(
        require_private_regular(&published),
        Err(PrivateFileError::Policy)
    ));
    assert!(matches!(
        open_private_regular(&path),
        Err(PrivateFileError::Policy)
    ));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    fs::remove_file(&link).unwrap();
    let mut original = open_private_regular(&path).unwrap();
    let replacement = directory.path().join("replacement");
    fs::write(&replacement, b"replacement").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let mut bytes = Vec::new();
    original.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original");
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}

#[cfg(unix)]
#[test]
fn private_open_refuses_final_links_directories_and_fifo_without_reading() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("identity");
    drop(
        AtomicPublication::stage_private(&target, b"identity")
            .unwrap()
            .publish_new()
            .unwrap(),
    );
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(
        open_private_regular(&link),
        Err(PrivateFileError::Open(BoundedReadError::NotRegular { .. }))
    ));
    assert!(matches!(
        open_private_regular(directory.path()),
        Err(PrivateFileError::Open(BoundedReadError::NotRegular { .. }))
    ));
    let fifo = directory.path().join("pipe");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    assert!(matches!(
        open_private_regular(&fifo),
        Err(PrivateFileError::Open(BoundedReadError::NotRegular { .. }))
    ));
}

#[cfg(not(unix))]
#[test]
fn private_open_refuses_unsupported_platform_before_opening() {
    assert!(matches!(
        open_private_regular(Path::new("not-opened")),
        Err(PrivateFileError::Unsupported)
    ));
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

#[test]
fn bounded_read_returns_a_regular_file_up_to_its_cap() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("package.toml");
    fs::write(&path, b"exactly 7").unwrap();
    assert_eq!(read_regular_bounded(&path, 9).unwrap(), b"exactly 7");
    assert_eq!(read_regular_bounded(&path, 4096).unwrap(), b"exactly 7");
    assert!(matches!(
        read_regular_bounded(&path, 8).unwrap_err(),
        BoundedReadError::TooLarge { cap: 8, .. }
    ));
    let empty = directory.path().join("empty");
    fs::write(&empty, b"").unwrap();
    assert!(read_regular_bounded(&empty, 0).unwrap().is_empty());
}

#[test]
fn bounded_read_reports_a_missing_file_with_its_kind() {
    let directory = tempfile::tempdir().unwrap();
    let error = read_regular_bounded(&directory.path().join("absent"), 16).unwrap_err();
    assert_eq!(error.io_kind(), Some(io::ErrorKind::NotFound));
}

#[test]
fn bounded_read_refuses_directories() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        read_regular_bounded(directory.path(), 16).unwrap_err(),
        BoundedReadError::NotRegular { .. } | BoundedReadError::Io { .. }
    ));
}

#[cfg(unix)]
#[test]
fn bounded_read_refuses_a_final_symlink_to_a_file_or_a_dangling_one() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    fs::write(&target, b"secret").unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(
        read_regular_bounded(&link, 64).unwrap_err(),
        BoundedReadError::NotRegular { .. }
    ));
    fs::remove_file(&target).unwrap();
    assert!(matches!(
        read_regular_bounded(&link, 64).unwrap_err(),
        BoundedReadError::NotRegular { .. }
    ));
}

#[cfg(unix)]
#[test]
fn bounded_read_follows_links_in_parent_directories_only() {
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("file"), b"content").unwrap();
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert_eq!(
        read_regular_bounded(&alias.join("file"), 64).unwrap(),
        b"content"
    );
}

#[cfg(unix)]
#[test]
fn bounded_read_refuses_a_fifo_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory.path().join("pipe");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    assert!(matches!(
        read_regular_bounded(&fifo, 64).unwrap_err(),
        BoundedReadError::NotRegular { .. }
    ));
}

#[cfg(unix)]
#[test]
fn bounded_read_refuses_devices_whose_size_is_unbounded() {
    assert!(matches!(
        read_regular_bounded(Path::new("/dev/zero"), 64).unwrap_err(),
        BoundedReadError::NotRegular { .. }
    ));
}
