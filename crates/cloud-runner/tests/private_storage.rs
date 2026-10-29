mod support;
use nocterm_cloud_model::journal::JournalError;
use nocterm_cloud_runner::JournalLimits;
use std::sync::Arc;
use support::*;

#[cfg(unix)]
#[test]
fn unsafe_root_database_and_sidecar_permissions_are_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = private_root();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        open_journal(
            root.path(),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    drop(store);
    let database = root.path().join("cloud-runs.sqlite3");
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        open_journal(
            root.path(),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(root.path().join("cloud-runs.sqlite3-journal"), []).unwrap();
    std::fs::set_permissions(
        root.path().join("cloud-runs.sqlite3-journal"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(matches!(
        open_journal(
            root.path(),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
}

#[cfg(unix)]
#[test]
fn linked_database_sidecar_or_ancestor_never_redirects_storage() {
    use std::os::unix::fs::symlink;
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let root = private_root();
        let destination = private_root();
        let victim = destination.path().join("victim");
        std::fs::write(&victim, b"preserve").unwrap();
        symlink(
            &victim,
            root.path().join(format!("cloud-runs.sqlite3{suffix}")),
        )
        .unwrap();
        assert!(matches!(
            open_journal(
                root.path(),
                Arc::new(TestCipher::default()),
                JournalLimits::default()
            ),
            Err(JournalError::Corrupt)
        ));
        assert_eq!(std::fs::read(victim).unwrap(), b"preserve");
    }
    let root = private_root();
    let parent = private_root();
    let link = parent.path().join("linked-root");
    symlink(root.path(), &link).unwrap();
    assert!(matches!(
        open_journal(
            &link,
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
}

#[cfg(unix)]
#[test]
fn hard_linked_database_is_refused_and_created_database_is_private() {
    use std::os::unix::fs::MetadataExt as _;
    let root = private_root();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    drop(store);
    let database = root.path().join("cloud-runs.sqlite3");
    assert_eq!(std::fs::metadata(&database).unwrap().mode() & 0o777, 0o600);
    std::fs::hard_link(&database, root.path().join("second-name")).unwrap();
    assert!(matches!(
        open_journal(
            root.path(),
            Arc::new(TestCipher::default()),
            JournalLimits::default()
        ),
        Err(JournalError::Corrupt)
    ));
}
