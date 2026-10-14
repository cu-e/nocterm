use nocterm_localfs::LocalFilesystem;
use nocterm_session::{
    fs::UploadMode,
    local_fs::{LocalFs, LocalFsError, PublicationOutcome},
};
use std::ffi::OsStr;

#[test]
fn staging_preserves_old_file_handles_collision_and_does_not_publish_on_drop() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"old").unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let directory = fs.open_directory(root.path()).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"new").unwrap();
    stage.sync().unwrap();
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"old");
    assert!(matches!(
        stage.publish(OsStr::new("file"), UploadMode::Create),
        Err(LocalFsError::AlreadyExists(_))
    ));
    assert!(matches!(
        stage
            .publish(OsStr::new("renamed"), UploadMode::Create)
            .unwrap(),
        PublicationOutcome::Published
    ));
    assert!(stage.write(b"after").is_err());
    assert!(
        stage
            .publish(OsStr::new("twice"), UploadMode::Create)
            .is_err()
    );
    drop(stage);
    assert_eq!(std::fs::read(root.path().join("renamed")).unwrap(), b"new");
    let mut replace = directory.stage().unwrap();
    replace.write(b"replace").unwrap();
    replace
        .publish(OsStr::new("file"), UploadMode::Replace)
        .unwrap();
    drop(replace);
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"replace");
    let mut abandoned = directory.stage().unwrap();
    abandoned.write(b"discard").unwrap();
    drop(abandoned);
    let canceled = directory.stage().unwrap();
    canceled.cancel().unwrap();
    assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(root.path().join("file")).unwrap().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn publishing_and_drop_cleanup_remain_inside_the_held_directory_after_a_swap() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    std::fs::create_dir(&selected).unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = fs.open_directory(&selected).unwrap();
    let mut stage = held.stage().unwrap();
    stage.write(b"held").unwrap();
    std::fs::rename(&selected, root.path().join("moved")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &selected).unwrap();
    stage
        .publish(OsStr::new("file"), UploadMode::Create)
        .unwrap();
    drop(stage);
    assert_eq!(
        std::fs::read(root.path().join("moved/file")).unwrap(),
        b"held"
    );
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    assert_eq!(
        std::fs::read_dir(root.path().join("moved"))
            .unwrap()
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn drop_after_replacing_never_unlinks_a_new_file_at_the_former_temporary_name() {
    published_stage_relinquishes_its_temporary_name(UploadMode::Replace);
}

#[cfg(unix)]
#[test]
fn create_publication_removes_its_temporary_link_and_drop_preserves_a_new_owner() {
    published_stage_relinquishes_its_temporary_name(UploadMode::Create);
}

#[cfg(unix)]
fn published_stage_relinquishes_its_temporary_name(mode: UploadMode) {
    let root = tempfile::tempdir().unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let directory = fs.open_directory(root.path()).unwrap();
    let mut stage = directory.stage().unwrap();
    let temporary = std::fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    stage.write(b"new bytes").unwrap();
    stage.publish(OsStr::new("final"), mode).unwrap();
    assert!(!temporary.exists(), "Committed stage relinquishes its name");
    std::fs::write(&temporary, b"external owner").unwrap();
    drop(stage);
    assert_eq!(std::fs::read(temporary).unwrap(), b"external owner");
    assert_eq!(
        std::fs::read(root.path().join("final")).unwrap(),
        b"new bytes"
    );
}

#[cfg(unix)]
#[test]
fn explicit_cancel_reports_permission_failure_and_preserves_unremoved_private_bytes() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let directory = filesystem.open_directory(root.path()).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"private download").unwrap();
    let temporary = std::fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let permissions = std::fs::metadata(root.path()).unwrap().permissions();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = stage.cancel();
    // Restore fixture cleanup authority before any assertion can unwind.
    std::fs::set_permissions(root.path(), permissions).unwrap();
    assert!(
        matches!(result, Err(LocalFsError::PermissionDenied(ref path)) if path == &temporary),
        "Explicit cancellation must report refused removal, not success: {result:?}"
    );
    assert_eq!(std::fs::read(temporary).unwrap(), b"private download");
    assert!(!root.path().join("final").exists());
}

#[cfg(unix)]
#[test]
fn explicit_cancel_accepts_a_candidate_already_removed_by_external_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let directory = filesystem.open_directory(root.path()).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"private download").unwrap();
    let temporary = std::fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::remove_file(&temporary).unwrap();
    stage.cancel().unwrap();
    assert!(!temporary.exists());
    assert!(!root.path().join("final").exists());
}

#[cfg(unix)]
#[test]
fn explicit_cancel_removes_only_its_held_candidate_after_ancestor_replacement() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    let moved = root.path().join("moved");
    std::fs::create_dir(&selected).unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let directory = filesystem.open_directory(&selected).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"discarded").unwrap();
    let name = std::fs::read_dir(&selected)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    std::fs::rename(&selected, &moved).unwrap();
    std::os::unix::fs::symlink(outside.path(), &selected).unwrap();
    std::fs::write(outside.path().join(&name), b"outside owner").unwrap();
    stage.cancel().unwrap();
    assert!(!moved.join(&name).exists());
    assert_eq!(
        std::fs::read(outside.path().join(&name)).unwrap(),
        b"outside owner"
    );
    assert!(!moved.join("final").exists());
    assert!(!outside.path().join("final").exists());
}

#[cfg(unix)]
#[test]
fn failed_replace_retains_native_cause_and_same_held_stage_for_retry() {
    use std::error::Error;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = fs.open_directory(root.path()).unwrap();
    let mut stage = held.stage().unwrap();
    stage.write(b"candidate").unwrap();
    let error = stage
        .publish(OsStr::new("directory"), UploadMode::Replace)
        .unwrap_err();
    assert!(root.path().join("directory").is_dir());
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(original.kind(), std::io::ErrorKind::IsADirectory);
    let clone = error.clone();
    assert!(std::ptr::eq(
        original,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
    assert!(matches!(
        stage
            .publish(OsStr::new("retried"), UploadMode::Create)
            .unwrap(),
        PublicationOutcome::Published
    ));
    drop(stage);
    assert_eq!(
        std::fs::read(root.path().join("retried")).unwrap(),
        b"candidate"
    );
}
