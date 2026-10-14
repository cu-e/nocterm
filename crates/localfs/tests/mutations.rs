use nocterm_localfs::LocalFilesystem;
use nocterm_session::local_fs::{CancelFlag, LocalFsError, LocalMutations, RemovalLimits};
use std::ffi::OsStr;

#[test]
fn no_replace_rename_permissions_and_recursive_remove_preserve_unselected_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), b"original").unwrap();
    std::fs::write(root.path().join("exists"), b"keep").unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    assert!(
        fs.rename(&root.path().join("source"), OsStr::new("exists"))
            .is_err()
    );
    assert_eq!(std::fs::read(root.path().join("exists")).unwrap(), b"keep");
    fs.rename(&root.path().join("source"), OsStr::new("renamed"))
        .unwrap();
    #[cfg(unix)]
    {
        fs.set_permissions(&root.path().join("renamed"), 0).unwrap();
        assert_eq!(
            fs.metadata(&root.path().join("renamed"))
                .unwrap()
                .permissions,
            Some(0)
        );
        fs.set_permissions(&root.path().join("renamed"), 0o640)
            .unwrap();
    }
    std::fs::create_dir_all(root.path().join("tree/nested/empty")).unwrap();
    std::fs::write(root.path().join("tree/nested/file"), b"delete").unwrap();
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(matches!(
        fs.remove(&root.path().join("tree"), RemovalLimits::default(), &cancel),
        Err(LocalFsError::Cancelled)
    ));
    let outcome = fs
        .remove(
            &root.path().join("tree"),
            RemovalLimits::default(),
            &CancelFlag::default(),
        )
        .unwrap();
    assert_eq!(outcome.removed, 4);
    assert!(!root.path().join("tree").exists());
    assert_eq!(
        std::fs::read(root.path().join("renamed")).unwrap(),
        b"original"
    );
    assert!(
        fs.remove(
            root.path().ancestors().last().unwrap(),
            RemovalLimits::default(),
            &CancelFlag::default()
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn final_links_are_unlinked_and_link_ancestors_never_redirect_mutations() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), b"keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("alias")).unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    assert!(
        fs.rename(&root.path().join("alias/keep"), OsStr::new("renamed"))
            .is_err()
    );
    assert!(
        fs.set_permissions(&root.path().join("alias"), 0o777)
            .is_err()
    );
    assert!(
        fs.remove(
            &root.path().join("alias/../victim"),
            RemovalLimits::default(),
            &CancelFlag::default()
        )
        .is_err()
    );
    fs.remove(
        &root.path().join("alias"),
        RemovalLimits::default(),
        &CancelFlag::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
}

#[test]
fn deletion_bound_reports_partial_mutation_and_preserves_unvisited_entries() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("tree")).unwrap();
    for name in ["one", "two", "three"] {
        std::fs::write(root.path().join("tree").join(name), b"data").unwrap();
    }
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let error = fs
        .remove(
            &root.path().join("tree"),
            RemovalLimits::new(10, 2).unwrap(),
            &CancelFlag::default(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LocalFsError::PartialRemoval { removed: 1, .. }
    ));
    use std::error::Error;
    let clone = error.clone();
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<LocalFsError>()
        .unwrap();
    assert!(matches!(original, LocalFsError::BoundExceeded("deletion")));
    let shared = clone
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<LocalFsError>()
        .unwrap();
    assert!(std::ptr::eq(original, shared));
    assert_eq!(
        std::fs::read_dir(root.path().join("tree")).unwrap().count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn native_non_utf8_names_are_never_round_tripped_through_lossy_display() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let name = OsStr::from_bytes(b"invalid-\xff");
    std::fs::write(root.path().join(name), b"exact").unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    fs.rename(&root.path().join(name), OsStr::from_bytes(b"renamed-\xfe"))
        .unwrap();
    assert_eq!(
        std::fs::read(root.path().join(OsStr::from_bytes(b"renamed-\xfe"))).unwrap(),
        b"exact"
    );
}
