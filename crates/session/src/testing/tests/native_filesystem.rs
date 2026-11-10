use crate::testing::{FsOperation, InMemoryFs};
use crate::{fs::UploadMode, local_fs::*};
use std::{ffi::OsStr, path::Path};

#[cfg(unix)]
#[test]
fn native_and_remote_ports_share_bytes_and_held_capabilities_survive_rename_and_alias_replacement()
{
    let fs = InMemoryFs::default();
    fs.seed_file("/selected/original", b"keep").unwrap();
    fs.seed_file("/outside/external", b"secret").unwrap();
    let held = LocalFs::open_directory(&fs, Path::new("/selected")).unwrap();
    let id = held.identity();
    LocalMutations::rename(&fs, Path::new("/selected"), OsStr::new("moved")).unwrap();
    fs.seed_native_alias(Path::new("/selected"), Path::new("/outside"))
        .unwrap();
    assert_eq!(
        id,
        LocalFs::open_directory(&fs, Path::new("/moved"))
            .unwrap()
            .identity()
    );
    assert_eq!(held.entries().unwrap().next().unwrap().unwrap(), "original");
    assert_eq!(
        LocalFs::browse(
            &fs,
            Path::new("/selected"),
            ListingLimits::default(),
            &CancelFlag::default()
        )
        .unwrap()
        .entries[0]
            .name,
        "external"
    );
    assert!(LocalFs::open_directory(&fs, Path::new("/selected")).is_err());
    let mut source = held.open_source(OsStr::new("original")).unwrap();
    assert_eq!(source.read(32).unwrap(), b"keep");
    source.close().unwrap();
    assert!(held.open_source(OsStr::new("../external")).is_err());
    let mut stage = held.stage().unwrap();
    stage.write(b"held").unwrap();
    stage
        .publish(OsStr::new("published"), UploadMode::Create)
        .unwrap();
    drop(stage);
    assert_eq!(fs.bytes("/moved/published").unwrap(), b"held");
    assert!(fs.bytes("/outside/published").is_err());
}

#[test]
fn local_stage_collisions_can_retry_same_candidate_and_committed_warnings_never_allow_second_publish()
 {
    let fs = InMemoryFs::default();
    let base = LocalFs::home(&fs).unwrap().unwrap();
    fs.seed_native_file(&base.join("existing"), b"old").unwrap();
    let directory = LocalFs::open_directory(&fs, &base).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"new").unwrap();
    stage.sync().unwrap();
    assert!(matches!(
        stage.publish(OsStr::new("existing"), UploadMode::Create),
        Err(LocalFsError::AlreadyExists(_))
    ));
    fs.fail_next_local(
        FsOperation::DirectorySync,
        base.clone(),
        LocalFsError::Unsupported("directory fsync"),
    );
    let PublicationOutcome::PublishedWithWarning(warning) = stage
        .publish(OsStr::new("renamed"), UploadMode::Create)
        .unwrap()
    else {
        panic!("injected directory sync failure is a committed warning")
    };
    use std::error::Error;
    let clone = warning.clone();
    let native = warning
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(native.kind(), std::io::ErrorKind::Other);
    assert_eq!(warning.kind(), native.kind());
    assert!(std::ptr::eq(
        native,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
    assert!(
        stage
            .publish(OsStr::new("again"), UploadMode::Create)
            .is_err()
    );
    drop(stage);
    let mut source = LocalFs::open_source(&fs, &base.join("renamed")).unwrap();
    assert_eq!(source.read(100).unwrap(), b"new");
    source.close().unwrap();
    assert_eq!(fs.activity().completed_uploads, 1);
    assert_eq!(fs.activity().live_uploads, 0);
}

#[test]
fn synchronous_local_failures_and_canceled_scans_need_no_clock_or_blocking_gate() {
    let fs = InMemoryFs::default();
    let base = LocalFs::home(&fs).unwrap().unwrap();
    fs.seed_native_file(&base.join("file"), b"abc").unwrap();
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = observed.clone();
    fs.observe_local(std::sync::Arc::new(move |operation, path: &Path| {
        log.lock().unwrap().push((operation, path.to_owned()))
    }));
    fs.fail_next_local(
        FsOperation::Read,
        base.join("file"),
        LocalFsError::Cancelled,
    );
    let mut source = LocalFs::open_source(&fs, &base.join("file")).unwrap();
    assert!(matches!(source.read(2), Err(LocalFsError::Cancelled)));
    assert_eq!(source.read(2).unwrap(), b"ab");
    drop(source);
    let cancel = CancelFlag::default();
    cancel.cancel();
    let stats = LocalFs::scan(
        &fs,
        &base,
        &ScanPolicy::new(vec![], 100, 256).unwrap(),
        &cancel,
        &|_| {},
    )
    .unwrap();
    assert!(!stats.complete);
    assert!(
        observed
            .lock()
            .unwrap()
            .iter()
            .any(|(operation, _)| *operation == FsOperation::Read)
    );
}

#[cfg(unix)]
#[test]
fn native_fixture_names_are_exact_even_when_their_display_is_lossy() {
    use std::os::unix::ffi::OsStrExt;
    let fs = InMemoryFs::default();
    let name = OsStr::from_bytes(b"name-\xff");
    let path = Path::new("/").join(name);
    fs.seed_native_file(&path, b"exact").unwrap();
    let listing = LocalFs::browse(
        &fs,
        Path::new("/"),
        ListingLimits::default(),
        &CancelFlag::default(),
    )
    .unwrap();
    assert_eq!(listing.entries[0].name, name);
    LocalMutations::rename(&fs, &path, OsStr::from_bytes(b"renamed-\xfe")).unwrap();
    let mut source = LocalFs::open_source(
        &fs,
        &Path::new("/").join(OsStr::from_bytes(b"renamed-\xfe")),
    )
    .unwrap();
    assert_eq!(source.read(100).unwrap(), b"exact");
}

#[cfg(unix)]
#[test]
fn read_only_parent_navigation_resolves_aliases_first_and_mutations_honor_capabilities() {
    let fs = InMemoryFs::default();
    fs.seed_native_directory(Path::new("/browse")).unwrap();
    fs.seed_native_directory(Path::new("/target")).unwrap();
    fs.seed_native_directory(Path::new("/target/child"))
        .unwrap();
    fs.seed_native_file(Path::new("/target/value"), b"kept".to_vec())
        .unwrap();
    fs.seed_native_alias(Path::new("/browse/alias"), Path::new("/target/child"))
        .unwrap();
    let listing = fs
        .browse(
            Path::new("/browse/alias/.."),
            ListingLimits::default(),
            &CancelFlag::default(),
        )
        .unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.name.as_os_str())
            .collect::<Vec<_>>(),
        [OsStr::new("child"), OsStr::new("value")]
    );
    assert!(matches!(
        fs.open_directory(Path::new("/browse/alias/..")),
        Err(LocalFsError::InvalidPath(_))
    ));
    fs.set_capabilities(crate::FsCapabilities::default());
    assert!(matches!(
        LocalMutations::metadata(&fs, Path::new("/target/value")),
        Err(LocalFsError::Unsupported(_))
    ));
    assert!(matches!(
        LocalMutations::rename(&fs, Path::new("/target/value"), OsStr::new("renamed")),
        Err(LocalFsError::Unsupported(_))
    ));
    assert!(matches!(
        LocalMutations::set_permissions(&fs, Path::new("/target/value"), 0),
        Err(LocalFsError::Unsupported(_))
    ));
    assert!(matches!(
        LocalMutations::remove(
            &fs,
            Path::new("/target/value"),
            RemovalLimits::default(),
            &CancelFlag::default()
        ),
        Err(LocalFsError::Unsupported(_))
    ));
    assert_eq!(
        fs.open_source(Path::new("/target/value"))
            .unwrap()
            .read(32)
            .unwrap(),
        b"kept"
    );
}

#[test]
fn native_links_keep_lstat_kind_while_completion_checks_directory_targets() {
    let fs = InMemoryFs::default();
    let base = fs.home().unwrap().unwrap();
    fs.seed_native_directory(&base.join("directory")).unwrap();
    fs.seed_native_alias(&base.join("dir-link"), &base.join("directory"))
        .unwrap();
    fs.seed_native_file(&base.join("file"), b"bytes").unwrap();
    fs.seed_native_alias(&base.join("file-link"), &base.join("file"))
        .unwrap();
    fs.seed_native_alias(&base.join("missing-link"), &base.join("missing"))
        .unwrap();
    let listing = fs
        .browse(&base, ListingLimits::default(), &CancelFlag::default())
        .unwrap();
    assert_eq!(listing.entries[0].name, "directory");
    for entry in listing
        .entries
        .iter()
        .filter(|entry| entry.metadata.is_symlink)
    {
        assert_eq!(entry.metadata.kind, crate::EntryKind::Other);
        assert_eq!(entry.completion_directory, entry.name == "dir-link");
        assert!(matches!(
            fs.open_source(&base.join(&entry.name)),
            Err(LocalFsError::LinkRefused(_))
        ));
    }
    LocalMutations::remove(
        &fs,
        &base.join("directory"),
        RemovalLimits::default(),
        &CancelFlag::default(),
    )
    .unwrap();
    fs.seed_native_file(&base.join("directory"), b"replacement")
        .unwrap();
    let updated = fs
        .browse(&base, ListingLimits::default(), &CancelFlag::default())
        .unwrap();
    assert!(
        !updated
            .entries
            .iter()
            .find(|entry| entry.name == "dir-link")
            .unwrap()
            .completion_directory
    );
}

#[test]
fn unreadable_native_children_cannot_extend_the_statistics_walk_budget() {
    let fs = InMemoryFs::default();
    let base = fs.home().unwrap().unwrap();
    let denied = base.join("a-denied");
    fs.seed_native_file(&denied, b"denied").unwrap();
    fs.seed_native_file(&base.join("b-excess"), b"must not be counted")
        .unwrap();
    fs.fail_next_local(
        FsOperation::Metadata,
        denied.clone(),
        LocalFsError::PermissionDenied(denied),
    );
    let result = fs
        .scan(
            &base,
            &ScanPolicy::new(vec![], 1, 256).unwrap(),
            &CancelFlag::default(),
            &|_| {},
        )
        .unwrap();
    assert_eq!((result.files, result.bytes, result.inaccessible), (0, 0, 2));
    assert!(matches!(
        result.errors.last(),
        Some(ScanIssue::Local(LocalFsError::BoundExceeded(
            "statistics entries"
        )))
    ));
}

#[test]
fn bounded_shared_local_removal_preserves_partial_cause_and_unvisited_bytes() {
    use std::error::Error;
    let fs = InMemoryFs::default();
    let base = fs.home().unwrap().unwrap();
    let tree = base.join("tree");
    for name in ["one", "two", "three"] {
        fs.seed_native_file(&tree.join(name), b"keep").unwrap();
    }
    let error = LocalMutations::remove(
        &fs,
        &tree,
        RemovalLimits::new(10, 2).unwrap(),
        &CancelFlag::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LocalFsError::PartialRemoval { removed: 1, .. }
    ));
    let clone = error.clone();
    let original = error
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<LocalFsError>()
        .unwrap();
    assert!(matches!(original, LocalFsError::BoundExceeded("deletion")));
    assert!(std::ptr::eq(
        original,
        clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<LocalFsError>()
            .unwrap()
    ));
    let remaining = fs
        .browse(&tree, ListingLimits::default(), &CancelFlag::default())
        .unwrap();
    assert_eq!(remaining.entries.len(), 2);
    for entry in remaining.entries {
        let mut source = fs.open_source(&tree.join(entry.name)).unwrap();
        assert_eq!(source.read(32).unwrap(), b"keep");
        source.close().unwrap();
    }
}

#[test]
fn held_leaf_removal_rejects_directories_and_keeps_renamed_parent_identity() {
    use crate::local_fs::{LocalFs as _, LocalMutations as _};
    use std::{ffi::OsStr, path::Path};
    let fs = crate::testing::InMemoryFs::default();
    fs.seed_native_directory(Path::new("/held")).unwrap();
    fs.seed_native_directory(Path::new("/held/child")).unwrap();
    fs.seed_native_file(Path::new("/held/leaf"), b"private".to_vec())
        .unwrap();
    let directory = fs.open_directory(Path::new("/held")).unwrap();
    assert!(directory.remove_file(OsStr::new("child")).is_err());
    for name in ["", ".", "..", "a/b"] {
        assert!(directory.remove_file(OsStr::new(name)).is_err());
    }
    fs.rename(Path::new("/held"), OsStr::new("moved")).unwrap();
    fs.seed_native_directory(Path::new("/held")).unwrap();
    fs.seed_native_file(Path::new("/held/leaf"), b"replacement".to_vec())
        .unwrap();
    directory.remove_file(OsStr::new("leaf")).unwrap();
    assert!(directory.metadata(OsStr::new("leaf")).unwrap().is_none());
    assert!(fs.open_source(Path::new("/held/leaf")).is_ok());
}
