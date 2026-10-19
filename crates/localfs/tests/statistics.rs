use nocterm_localfs::LocalFilesystem;
use nocterm_session::local_fs::{CancelFlag, LocalFs, ScanPolicy};

#[test]
fn counts_immediate_children_recursive_bytes_and_excludes_named_subtrees() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("node_modules/pkg")).unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("a"), b"123").unwrap();
    std::fs::write(root.path().join("src/main"), b"45678").unwrap();
    std::fs::write(root.path().join("node_modules/pkg/ignored"), [0; 4096]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let policy = ScanPolicy::new(vec!["node_modules".into()], 1000, 256).unwrap();
    let progress = std::sync::Mutex::new(Vec::new());
    let result = fs
        .scan(
            root.path(),
            &policy,
            &CancelFlag::default(),
            &|statistics| progress.lock().unwrap().push(statistics),
        )
        .unwrap();
    assert_eq!(
        (
            result.files,
            result.directories,
            result.bytes,
            result.excluded
        ),
        (1, 2, 8, 1)
    );
    assert!(result.complete);
    assert!(progress.lock().unwrap().last().unwrap().complete);
}

#[test]
fn canceled_missing_and_deep_scans_have_explicit_partial_results() {
    let root = tempfile::tempdir().unwrap();
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let policy = ScanPolicy::new(vec![], 1000, 2).unwrap();
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(
        !fs.scan(root.path(), &policy, &cancel, &|_| {})
            .unwrap()
            .complete
    );
    let missing = fs
        .scan(
            &root.path().join("missing"),
            &policy,
            &CancelFlag::default(),
            &|_| {},
        )
        .unwrap();
    assert!(missing.complete);
    assert_eq!(missing.inaccessible, 1);
    std::fs::create_dir_all(root.path().join("a/b/c")).unwrap();
    std::fs::write(root.path().join("a/b/c/unvisited"), b"private").unwrap();
    let result = fs
        .scan(root.path(), &policy, &CancelFlag::default(), &|_| {})
        .unwrap();
    assert!(result.complete);
    assert_eq!(result.bytes, 0);
    assert_eq!(result.inaccessible, 1);
}

#[cfg(unix)]
#[test]
fn skipped_symlinks_cannot_bypass_the_entry_walk_limit() {
    let root = tempfile::tempdir().unwrap();
    for name in ["one", "two", "three"] {
        std::os::unix::fs::symlink(root.path(), root.path().join(name)).unwrap();
    }
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let result = fs
        .scan(
            root.path(),
            &ScanPolicy::new(vec![], 1, 256).unwrap(),
            &CancelFlag::default(),
            &|_| {},
        )
        .unwrap();
    assert_eq!((result.files, result.directories, result.bytes), (0, 0, 0));
    assert_eq!(result.inaccessible, 1);
    assert!(result.complete);
}
