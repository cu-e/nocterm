//! File operations against a real server, with local observations of effects.
use super::*;
use nocterm_session::{FsCapabilities, FsError};

#[tokio::test]
async fn detailed_metadata_keeps_optional_posix_fields_and_does_not_follow_links() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("source");
    fs::write(&source, b"metadata-bytes").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let directory = sshd.root().join("folder");
    fs::create_dir(&directory).unwrap();
    let link = sshd.root().join("link");
    symlink(&source, &link).unwrap();
    let dir_link = sshd.root().join("dir-link");
    symlink(&directory, &dir_link).unwrap();
    let dangling = sshd.root().join("dangling");
    symlink(sshd.root().join("missing"), &dangling).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    assert_eq!(
        remote.capabilities(),
        FsCapabilities {
            metadata: true,
            rename: true,
            remove: true,
            set_permissions: true
        }
    );
    let actual = remote.metadata(source.to_str().unwrap()).await.unwrap();
    let expected = fs::metadata(&source).unwrap();
    assert_eq!(actual.kind, EntryKind::File);
    assert!(!actual.is_symlink);
    assert_eq!(actual.size, Some(expected.len()));
    assert_eq!(actual.permissions, Some(0o640));
    assert_eq!(actual.uid, Some(expected.uid()));
    assert_eq!(actual.gid, Some(expected.gid()));
    assert_eq!(actual.modified, Some(expected.mtime() as u64));
    for link in [&link, &dangling] {
        let actual = remote.metadata(link.to_str().unwrap()).await.unwrap();
        assert!(actual.is_symlink);
        assert_eq!(actual.kind, EntryKind::Other);
        assert_eq!(actual.size, Some(fs::symlink_metadata(link).unwrap().len()));
    }
    assert!(
        remote
            .metadata(&format!("{}/", dir_link.display()))
            .await
            .unwrap()
            .is_symlink
    );
    assert_eq!(
        remote
            .metadata(directory.to_str().unwrap())
            .await
            .unwrap()
            .kind,
        EntryKind::Directory
    );
    assert!(matches!(
        remote
            .metadata(sshd.root().join("not-found").to_str().unwrap())
            .await,
        Err(FsError::NotFound { .. })
    ));
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn rename_refuses_collisions_and_moves_files_directories_and_links_without_clobber() {
    use std::os::unix::fs::symlink;
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("source");
    let existing = sshd.root().join("existing");
    let renamed = sshd.root().join("renamed");
    fs::write(&source, b"selected").unwrap();
    fs::write(&existing, b"preserved").unwrap();
    let directory = sshd.root().join("folder");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child"), b"child").unwrap();
    let link = sshd.root().join("link");
    symlink(&source, &link).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    assert!(matches!(
        remote
            .rename(source.to_str().unwrap(), existing.to_str().unwrap())
            .await,
        Err(FsError::AlreadyExists { .. })
    ));
    assert_eq!(fs::read(&existing).unwrap(), b"preserved");
    assert_eq!(fs::read(&source).unwrap(), b"selected");
    remote
        .rename(source.to_str().unwrap(), renamed.to_str().unwrap())
        .await
        .unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&renamed).unwrap(), b"selected");
    let moved = sshd.root().join("moved");
    remote
        .rename(directory.to_str().unwrap(), moved.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(fs::read(moved.join("child")).unwrap(), b"child");
    let link_new = sshd.root().join("link-new");
    remote
        .rename(link.to_str().unwrap(), link_new.to_str().unwrap())
        .await
        .unwrap();
    assert!(fs::symlink_metadata(&link_new).unwrap().is_symlink());
    assert_eq!(fs::read_link(link_new).unwrap(), source);
    assert!(matches!(
        remote
            .rename(
                sshd.root().join("missing").to_str().unwrap(),
                sshd.root().join("new").to_str().unwrap()
            )
            .await,
        Err(FsError::NotFound { .. })
    ));
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn removal_is_one_entry_at_a_time_and_links_never_remove_their_target() {
    use std::os::unix::fs::symlink;
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("source");
    fs::write(&source, b"target").unwrap();
    let folder = sshd.root().join("nonempty");
    fs::create_dir(&folder).unwrap();
    let child = folder.join("child");
    fs::write(&child, b"keep").unwrap();
    let empty = sshd.root().join("empty-delete");
    fs::create_dir(&empty).unwrap();
    let link = sshd.root().join("link");
    symlink(&source, &link).unwrap();
    let dir_link = sshd.root().join("dir-link");
    symlink(&folder, &dir_link).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    assert!(remote.remove_file(folder.to_str().unwrap()).await.is_err());
    assert!(remote.remove_dir(source.to_str().unwrap()).await.is_err());
    assert!(remote.remove_dir(folder.to_str().unwrap()).await.is_err());
    assert_eq!(fs::read(&child).unwrap(), b"keep");
    assert!(remote.remove_dir(dir_link.to_str().unwrap()).await.is_err());
    remote.remove_file(link.to_str().unwrap()).await.unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"target");
    assert!(fs::symlink_metadata(&link).is_err());
    remote
        .remove_file(dir_link.to_str().unwrap())
        .await
        .unwrap();
    assert!(folder.is_dir());
    remote.remove_dir(empty.to_str().unwrap()).await.unwrap();
    assert!(!empty.exists());
    remote.remove_file(child.to_str().unwrap()).await.unwrap();
    remote.remove_dir(folder.to_str().unwrap()).await.unwrap();
    assert!(!folder.exists());
    assert!(matches!(
        remote.remove_file(child.to_str().unwrap()).await,
        Err(FsError::NotFound { .. })
    ));
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn permissions_change_only_mode_and_refuse_links_invalid_bits_and_dangerous_paths() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("source");
    fs::write(&source, b"unchanged bytes").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    let original = fs::metadata(&source).unwrap();
    let link = sshd.root().join("link");
    symlink(&source, &link).unwrap();
    let directory = sshd.root().join("folder");
    fs::create_dir(&directory).unwrap();
    let dir_link = sshd.root().join("dir-link");
    symlink(&directory, &dir_link).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    assert!(matches!(
        remote.set_permissions(link.to_str().unwrap(), 0o777).await,
        Err(FsError::Unsupported(_))
    ));
    assert!(matches!(
        remote
            .set_permissions(&format!("{}/", dir_link.display()), 0o777)
            .await,
        Err(FsError::Unsupported(_))
    ));
    assert_eq!(
        fs::metadata(&source).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert!(
        remote
            .set_permissions(source.to_str().unwrap(), 0o100000)
            .await
            .is_err()
    );
    remote
        .set_permissions(source.to_str().unwrap(), 0o604)
        .await
        .unwrap();
    let changed = fs::metadata(&source).unwrap();
    assert_eq!(changed.permissions().mode() & 0o7777, 0o604);
    assert_eq!(changed.uid(), original.uid());
    assert_eq!(changed.gid(), original.gid());
    assert_eq!(changed.mtime(), original.mtime());
    assert_eq!(fs::read(&source).unwrap(), b"unchanged bytes");
    remote
        .set_permissions(directory.to_str().unwrap(), 0o700)
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(directory).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    for path in [
        "/",
        "////",
        "/a/.",
        "/a/..",
        "/a/../b",
        "relative",
        "/nul\0path",
    ] {
        assert!(remote.remove_file(path).await.is_err());
        assert!(remote.remove_dir(path).await.is_err());
        assert!(remote.set_permissions(path, 0o644).await.is_err());
        assert!(remote.rename(path, source.to_str().unwrap()).await.is_err());
        assert!(remote.rename(source.to_str().unwrap(), path).await.is_err());
    }
    assert!(sshd.root().is_dir());
    assert_eq!(fs::read(&source).unwrap(), b"unchanged bytes");
    session.close();
    closed(&session, no_prompts).await;
}

#[tokio::test]
async fn disconnected_file_operations_fail_instead_of_touching_the_filesystem() {
    let Some(sshd) = Sshd::start() else { return };
    let source = sshd.root().join("source");
    let moved = sshd.root().join("moved");
    fs::write(&source, b"preserved").unwrap();
    let directory = sshd.root().join("empty-delete");
    fs::create_dir(&directory).unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    remote.metadata(source.to_str().unwrap()).await.unwrap();
    session.close();
    closed(&session, no_prompts).await;
    assert_eq!(remote.capabilities(), FsCapabilities::default());
    assert!(matches!(
        remote.metadata(source.to_str().unwrap()).await,
        Err(FsError::NotConnected)
    ));
    assert!(matches!(
        remote
            .rename(source.to_str().unwrap(), moved.to_str().unwrap())
            .await,
        Err(FsError::NotConnected)
    ));
    assert!(matches!(
        remote.remove_file(source.to_str().unwrap()).await,
        Err(FsError::NotConnected)
    ));
    assert!(matches!(
        remote.remove_dir(directory.to_str().unwrap()).await,
        Err(FsError::NotConnected)
    ));
    assert!(matches!(
        remote
            .set_permissions(source.to_str().unwrap(), 0o777)
            .await,
        Err(FsError::NotConnected)
    ));
    assert_eq!(fs::read(source).unwrap(), b"preserved");
    assert!(directory.is_dir());
    assert!(!moved.exists());
}

#[tokio::test]
async fn permission_denial_preserves_entries_and_reports_the_requested_path() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(sshd) = Sshd::start() else { return };
    // Root bypasses Unix DAC. All CI/dev non-root runs execute the denial;
    // privileged fixture runs cannot manufacture that OS-level precondition.
    if fs::metadata(sshd.root()).unwrap().uid() == 0 {
        return;
    }
    let locked = sshd.root().join("locked");
    fs::create_dir(&locked).unwrap();
    let source = locked.join("file");
    fs::write(&source, b"preserved").unwrap();
    let transport = sshd.transport("plain");
    let session = transport.open(sshd.request());
    connect(&session, accept).await;
    let remote = session.fs().unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let read = remote.metadata(source.to_str().unwrap()).await;
    let remove = remote.remove_file(source.to_str().unwrap()).await;
    let rename = remote
        .rename(
            source.to_str().unwrap(),
            sshd.root().join("moved").to_str().unwrap(),
        )
        .await;
    let chmod = remote
        .set_permissions(source.to_str().unwrap(), 0o777)
        .await;
    // Restore the fixture before assertions so an unexpected result cannot
    // prevent TempDir cleanup or leave an inaccessible test directory behind.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    for result in [read.map(|_| ()), remove, rename, chmod] {
        assert_eq!(
            result,
            Err(FsError::PermissionDenied {
                path: source.to_string_lossy().into_owned()
            })
        );
    }
    assert_eq!(fs::read(source).unwrap(), b"preserved");
    assert!(!sshd.root().join("moved").exists());
    session.close();
    closed(&session, no_prompts).await;
}
