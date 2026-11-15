#![cfg(windows)]

use nocterm_localfs::LocalFilesystem;
use nocterm_session::{
    fs::UploadMode,
    local_fs::{LocalCursor, LocalFs},
};
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs::OpenOptions,
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    process::Command,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ};

#[test]
fn native_identity_and_cursors_retain_the_selected_directory_and_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let ancestor = root.path().join("ancestor");
    let selected = ancestor.join("selected");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::create_dir(ancestor.join("sibling")).unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = filesystem.open_directory(&selected).unwrap();
    assert!(held.identity().is_held());
    assert_eq!(
        held.identity(),
        filesystem.open_directory(&selected).unwrap().identity()
    );
    assert_ne!(
        held.identity(),
        filesystem
            .open_directory(&ancestor.join("sibling"))
            .unwrap()
            .identity()
    );
    assert!(std::fs::rename(&ancestor, root.path().join("moved")).is_err());
    assert!(std::fs::rename(&selected, ancestor.join("moved")).is_err());
    assert!(std::fs::remove_dir(&selected).is_err());
    let mut cursor = held.entries().unwrap();
    drop(held);
    assert!(std::fs::rename(&ancestor, root.path().join("moved")).is_err());
    assert!(cursor.next().unwrap().is_none());
    drop(cursor);
    std::fs::rename(&ancestor, root.path().join("moved")).unwrap();
    assert!(root.path().join("moved/selected").is_dir());
}

#[test]
fn concurrent_native_cursors_have_independent_enumeration_positions() {
    let root = tempfile::tempdir().unwrap();
    // Long names require several native enumeration buffers, exposing a shared
    // enumeration position even when the iterator prefetches its first buffer.
    let expected: BTreeSet<OsString> = (0..256)
        .map(|index| OsString::from(format!("entry-{index:03}-{}", "x".repeat(120))))
        .collect();
    for name in &expected {
        std::fs::write(root.path().join(name), b"").unwrap();
    }
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = filesystem.open_directory(root.path()).unwrap();
    let mut first = held.entries().unwrap();
    let first_name = first.next().unwrap().unwrap();
    let mut second = held.entries().unwrap();
    let second_name = second.next().unwrap().unwrap();
    let mut first_names = collect_names(first.as_mut());
    first_names.insert(first_name);
    let mut second_names = collect_names(second.as_mut());
    second_names.insert(second_name);
    assert_eq!(first_names, expected);
    assert_eq!(second_names, expected);
}

#[test]
fn strict_capabilities_refuse_junctions_without_touching_their_targets() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"outside owner").unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let alias = root.path().join("junction");
    junction(&alias, outside.path());
    let held = filesystem.open_directory(root.path()).unwrap();
    assert!(filesystem.open_directory(&alias).is_err());
    assert!(held.child(OsStr::new("junction")).is_err());
    assert!(held.create_child(OsStr::new("junction")).is_err());
    assert!(filesystem.open_source(&alias.join("secret")).is_err());
    assert!(held.open_source(OsStr::new("junction")).is_err());
    assert!(
        held.metadata(OsStr::new("junction"))
            .unwrap()
            .unwrap()
            .is_symlink
    );
    held.remove_file(OsStr::new("junction")).unwrap();
    assert!(!alias.exists());
    assert_eq!(
        std::fs::read(outside.path().join("secret")).unwrap(),
        b"outside owner"
    );
}

#[test]
fn replacement_denied_by_an_open_reader_retains_the_stage_and_original_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = root.path().join("snapshot");
    std::fs::write(&snapshot, b"acknowledged").unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = filesystem.open_directory(root.path()).unwrap();
    let mut stage = held.stage().unwrap();
    stage.write(b"replacement").unwrap();
    stage.sync().unwrap();
    let reader = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&snapshot)
        .unwrap();
    assert!(
        stage
            .publish(OsStr::new("snapshot"), UploadMode::Replace)
            .is_err()
    );
    assert_eq!(std::fs::read(&snapshot).unwrap(), b"acknowledged");
    drop(reader);
    let mut old_reader = std::fs::File::open(&snapshot).unwrap();
    let mut old_bytes = Vec::new();
    old_reader.read_to_end(&mut old_bytes).unwrap();
    assert_eq!(old_bytes, b"acknowledged");
    drop(old_reader);
    stage
        .publish(OsStr::new("snapshot"), UploadMode::Replace)
        .unwrap();
    drop(stage);
    assert_eq!(std::fs::read(&snapshot).unwrap(), b"replacement");
    assert_eq!(
        collect_names(held.entries().unwrap().as_mut()),
        BTreeSet::from([OsString::from("snapshot")])
    );
}

#[test]
fn stages_retain_ancestry_until_cancel_or_drop_finishes_private_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("selected");
    std::fs::create_dir(&selected).unwrap();
    let filesystem = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let held = filesystem.open_directory(&selected).unwrap();
    let mut canceled = held.stage().unwrap();
    canceled.write(b"canceled").unwrap();
    let guard = std::fs::read_dir(&selected)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "guard")
        })
        .expect("a live stage keeps a protected child in the leaf directory");
    assert!(std::fs::remove_file(&guard).is_err());
    assert!(std::fs::rename(&guard, selected.join("moved-guard")).is_err());
    let mut abandoned = held.stage().unwrap();
    abandoned.write(b"abandoned").unwrap();
    drop(held);
    assert!(std::fs::rename(&selected, root.path().join("moved")).is_err());
    canceled.cancel().unwrap();
    assert!(std::fs::rename(&selected, root.path().join("moved")).is_err());
    drop(abandoned);
    assert!(std::fs::read_dir(&selected).unwrap().next().is_none());
    std::fs::rename(&selected, root.path().join("moved")).unwrap();
}

fn collect_names(cursor: &mut dyn LocalCursor) -> BTreeSet<OsString> {
    let mut names = BTreeSet::new();
    while let Some(name) = cursor.next().unwrap() {
        assert!(names.insert(name), "enumeration must not repeat an entry");
    }
    names
}

fn junction(link: &Path, target: &Path) {
    let output = Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = std::fs::symlink_metadata(link)
        .expect("mklink must create a real junction before checking capability rejection");
    assert_ne!(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT, 0);
}
