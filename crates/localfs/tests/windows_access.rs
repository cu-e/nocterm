//! Real ACL fixtures: only explicit denies on owned temporary paths are changed.
#![cfg(windows)]

use nocterm_localfs::LocalFilesystem;
use nocterm_session::{
    fs::UploadMode,
    local_fs::{LocalFs, LocalFsError},
};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn native_acl_child_storage_does_not_require_listing_its_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("ancestor");
    let child = parent.join("history");
    std::fs::create_dir_all(&child).unwrap();
    // RD is FILE_LIST_DIRECTORY on this directory alone, not inherited by children.
    let _deny = Deny::new(&parent, "RD");
    assert_eq!(
        std::fs::read_dir(&parent).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let fs = LocalFilesystem::with_base(child.clone()).unwrap();
    let ancestor = fs.open_directory(&parent).unwrap();
    assert!(std::fs::rename(&parent, root.path().join("moved")).is_err());
    assert!(matches!(
        ancestor.entries(),
        Err(LocalFsError::PermissionDenied(_))
    ));
    let held = fs.open_directory(&child).unwrap();
    let mut stage = held.stage().unwrap();
    stage.write(b"acknowledged").unwrap();
    stage
        .publish(OsStr::new("snapshot"), UploadMode::Create)
        .unwrap();
    let mut source = held.open_source(OsStr::new("snapshot")).unwrap();
    assert_eq!(source.read(32).unwrap(), b"acknowledged");
    assert_eq!(held.entries().unwrap().next().unwrap().unwrap(), "snapshot");
}

#[test]
fn native_acl_source_reads_do_not_require_reading_extended_attributes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"data").unwrap();
    // GENERIC_READ includes READ_EA, but a regular source never reads EAs.
    let _deny = Deny::new(&source, "REA");
    assert_eq!(
        std::fs::File::open(&source).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let fs = LocalFilesystem::with_base(root.path().to_owned()).unwrap();
    let mut held = fs.open_source(&source).unwrap();
    assert_eq!(held.metadata().unwrap().size, Some(4));
    assert_eq!(held.read(32).unwrap(), b"data");
}

struct Deny {
    path: PathBuf,
    sid: String,
}
impl Deny {
    fn new(path: &Path, permission: &str) -> Self {
        let output = Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        assert!(output.status.success());
        // The localized account name can use the console code page. Only the
        // ASCII SID after the final comma is used to identify the ACL trustee.
        let sid = String::from_utf8_lossy(&output.stdout)
            .trim()
            .rsplit(',')
            .next()
            .unwrap()
            .trim_matches('"')
            .to_owned();
        assert!(
            sid.starts_with("S-1-"),
            "whoami must return a real Windows SID"
        );
        let deny = Self {
            path: path.to_owned(),
            sid,
        };
        let output = Command::new("icacls.exe")
            .arg(path)
            .args(["/deny", &format!("*{}:({permission})", deny.sid)])
            .output()
            .unwrap();
        assert!(output.status.success(), "icacls /deny failed: {output:?}");
        deny
    }
}
impl Drop for Deny {
    fn drop(&mut self) {
        let result = Command::new("icacls.exe")
            .arg(&self.path)
            .args(["/remove:d", &format!("*{}", self.sid)])
            .output();
        if !result.as_ref().is_ok_and(|output| output.status.success()) {
            tracing::error!(
                path = ?self.path, error = ?result,
                "could not restore temporary fixture ACL",
            );
            assert!(
                std::thread::panicking(),
                "temporary ACL cleanup failed: {result:?}"
            );
        }
    }
}
