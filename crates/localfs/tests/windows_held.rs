//! Windows strict capabilities hold their ancestry and refuse reparse points.
#![cfg(windows)]
use nocterm_localfs::LocalFilesystem;
use nocterm_session::{
    fs::UploadMode,
    local_fs::{LocalDirectory, LocalFs, LocalFsError, PublicationOutcome},
};
use std::{ffi::OsStr, path::Path, process::Command, sync::Arc};

fn filesystem(root: &Path) -> LocalFilesystem {
    LocalFilesystem::with_base(root.to_owned()).unwrap()
}
fn junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed");
}
fn publish(directory: &Arc<dyn LocalDirectory>, name: &str, bytes: &[u8]) {
    let mut stage = directory.stage().unwrap();
    stage.write(bytes).unwrap();
    stage.sync().unwrap();
    assert!(matches!(
        stage
            .publish(OsStr::new(name), UploadMode::Replace)
            .unwrap(),
        PublicationOutcome::Published | PublicationOutcome::PublishedWithWarning(_)
    ));
}

#[test]
fn strict_directories_have_native_identity_and_pin_every_ancestor() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    std::fs::create_dir_all(root_path.join("a").join("b")).unwrap();
    let fs = filesystem(&root_path);
    let held = fs.open_directory(&root_path.join("a").join("b")).unwrap();
    assert!(held.identity().is_held());
    let child = held.create_child(OsStr::new("c")).unwrap();
    assert!(child.identity().is_held());
    assert_ne!(held.identity(), child.identity());
    assert_eq!(
        fs.open_directory(&root_path.join("a").join("b").join("c"))
            .unwrap()
            .identity(),
        child.identity()
    );
    // Neither the leaf nor an ancestor can be renamed away while it is held.
    assert!(
        std::fs::rename(
            root_path.join("a").join("b").join("c"),
            root_path.join("moved-c")
        )
        .is_err()
    );
    assert!(std::fs::rename(root_path.join("a"), root_path.join("moved-a")).is_err());
    drop(child);
    drop(held);
    std::fs::rename(root_path.join("a"), root_path.join("moved-a")).unwrap();
}

#[test]
fn junctions_are_refused_as_roots_ancestors_and_generated_children() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    std::fs::write(outside.path().join("secret.json"), b"secret").unwrap();
    junction(&root_path.join("link"), outside.path());
    let fs = filesystem(&root_path);
    assert!(matches!(
        fs.open_directory(&root_path.join("link")),
        Err(LocalFsError::LinkRefused(_))
    ));
    assert!(matches!(
        fs.open_directory(&root_path.join("link").join("inner")),
        Err(LocalFsError::LinkRefused(_) | LocalFsError::NotFound(_))
    ));
    let directory = fs.open_directory(&root_path).unwrap();
    for create in [false, true] {
        let opened = if create {
            directory.create_child(OsStr::new("link"))
        } else {
            directory.child(OsStr::new("link"))
        };
        assert!(matches!(opened, Err(LocalFsError::LinkRefused(_))));
    }
    assert!(
        directory
            .metadata(OsStr::new("link"))
            .unwrap()
            .unwrap()
            .is_symlink
    );
    assert!(matches!(
        directory.open_source(OsStr::new("link")),
        Err(LocalFsError::NotRegular(_) | LocalFsError::LinkRefused(_))
    ));
    assert_eq!(
        std::fs::read(outside.path().join("secret.json")).unwrap(),
        b"secret"
    );
}

#[test]
fn staged_publication_replaces_atomically_and_cancel_leaves_the_old_document() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let fs = filesystem(&root_path);
    let directory = fs
        .open_directory(&root_path)
        .unwrap()
        .create_child(OsStr::new("history"))
        .unwrap();
    publish(&directory, "chat.json", b"first");
    publish(&directory, "chat.json", b"second");
    assert_eq!(
        std::fs::read(root_path.join("history").join("chat.json")).unwrap(),
        b"second"
    );
    let mut stage = directory.stage().unwrap();
    stage.write(b"interrupted").unwrap();
    stage.cancel().unwrap();
    assert_eq!(
        std::fs::read(root_path.join("history").join("chat.json")).unwrap(),
        b"second"
    );
    let mut names = Vec::new();
    let mut entries = directory.entries().unwrap();
    while let Some(name) = entries.next().unwrap() {
        names.push(name);
    }
    assert_eq!(names, vec![std::ffi::OsString::from("chat.json")]);
    let mut source = directory.open_source(OsStr::new("chat.json")).unwrap();
    assert_eq!(source.read(1024).unwrap(), b"second");
    directory.remove_file(OsStr::new("chat.json")).unwrap();
    assert!(
        directory
            .metadata(OsStr::new("chat.json"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_read_only_target_is_reported_as_a_failed_write_without_losing_the_old_document() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let fs = filesystem(&root_path);
    let directory = fs.open_directory(&root_path).unwrap();
    publish(&directory, "chat.json", b"kept");
    let target = root_path.join("chat.json");
    let original = std::fs::metadata(&target).unwrap().permissions();
    let mut read_only = original.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&target, read_only).unwrap();
    let mut stage = directory.stage().unwrap();
    stage.write(b"replacement").unwrap();
    let refused = stage.publish(OsStr::new("chat.json"), UploadMode::Replace);
    assert!(refused.is_err());
    stage.cancel().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"kept");
    std::fs::set_permissions(&target, original).unwrap();
}
