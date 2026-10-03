//! Local mutations run on the background executor. Unix operations retain
//! directory capabilities and never follow an ancestor or final symlink.
use nocterm_session::{EntryKind, FileMetadata, FsError};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn error(path: &Path, e: impl std::fmt::Display) -> FsError {
    FsError::Other(format!("{}: {e}", path.display()))
}
fn cancelled(cancel: &AtomicBool) -> Result<(), FsError> {
    if cancel.load(Ordering::Acquire) {
        Err(FsError::Other(
            "Deletion cancelled; already deleted entries cannot be restored.".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(super) mod unix {
    use super::*;
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
    use std::{
        ffi::{OsStr, OsString},
        os::{fd::OwnedFd, unix::ffi::OsStrExt},
        path::{Component, PathBuf},
    };

    struct Parent {
        fd: OwnedFd,
        name: OsString,
    }
    fn parent(path: &Path) -> Result<Parent, FsError> {
        let absolute = std::path::absolute(path).map_err(|e| error(path, e))?;
        let mut normalized = PathBuf::from("/");
        for component in absolute.components() {
            match component {
                Component::Normal(name) => normalized.push(name),
                Component::ParentDir => {
                    return Err(error(
                        path,
                        "parent components are not accepted for mutations; navigate to the directory first",
                    ));
                }
                Component::RootDir | Component::CurDir => {}
                _ => return Err(error(path, "unsupported path prefix")),
            }
        }
        let name = normalized
            .file_name()
            .ok_or_else(|| error(path, "the filesystem root cannot be changed"))?
            .to_owned();
        let fd = open_directory(normalized.parent().unwrap())?;
        Ok(Parent { fd, name })
    }
    fn open_directory(path: &Path) -> Result<OwnedFd, FsError> {
        let absolute = std::path::absolute(path).map_err(|e| error(path, e))?;
        if absolute
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(error(
                path,
                "parent components are not accepted; navigate to the directory first",
            ));
        }
        let mut fd = fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| error(path, e))?;
        for component in absolute.components() {
            if let Component::Normal(name) = component {
                fd = fs::openat(
                    &fd,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| error(path, e))?;
            }
        }
        Ok(fd)
    }
    pub(crate) struct ScanDirectory {
        fd: OwnedFd,
        entries: fs::Dir,
    }
    impl ScanDirectory {
        pub(crate) fn open(path: &Path) -> Result<Self, FsError> {
            Self::from_fd(open_directory(path)?)
        }
        fn from_fd(fd: OwnedFd) -> Result<Self, FsError> {
            let entries =
                fs::Dir::read_from(&fd).map_err(|error| FsError::Other(error.to_string()))?;
            Ok(Self { fd, entries })
        }
        pub(crate) fn next(&mut self) -> Option<Result<OsString, FsError>> {
            loop {
                let entry = self.entries.next()?;
                match entry {
                    Ok(entry) => {
                        let name = OsStr::from_bytes(entry.file_name().to_bytes());
                        if name == "." || name == ".." {
                            continue;
                        }
                        return Some(Ok(name.to_owned()));
                    }
                    Err(error) => return Some(Err(FsError::Other(error.to_string()))),
                }
            }
        }
        pub(crate) fn metadata(&self, name: &OsStr) -> Result<FileMetadata, FsError> {
            fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map(metadata_from_stat)
                .map_err(|error| FsError::Other(error.to_string()))
        }
        pub(crate) fn child(&self, name: &OsStr) -> Result<Self, FsError> {
            let fd = fs::openat(
                &self.fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| FsError::Other(error.to_string()))?;
            Self::from_fd(fd)
        }
    }
    // libc stat typedef widths differ between Unix platforms.
    #[allow(clippy::unnecessary_cast)]
    fn metadata_from_stat(stat: fs::Stat) -> FileMetadata {
        let ty = FileType::from_raw_mode(stat.st_mode);
        FileMetadata {
            kind: match ty {
                FileType::Directory => EntryKind::Directory,
                FileType::RegularFile => EntryKind::File,
                _ => EntryKind::Other,
            },
            is_symlink: ty == FileType::Symlink,
            size: u64::try_from(stat.st_size).ok(),
            permissions: Some((stat.st_mode as u32) & 0o7777),
            uid: Some(stat.st_uid as u32),
            gid: Some(stat.st_gid as u32),
            modified: u64::try_from(stat.st_mtime).ok(),
        }
    }
    pub(super) fn metadata(path: &Path) -> Result<FileMetadata, FsError> {
        let p = parent(path)?;
        fs::statat(&p.fd, &p.name, AtFlags::SYMLINK_NOFOLLOW)
            .map(metadata_from_stat)
            .map_err(|e| error(path, e))
    }
    pub(super) fn rename(path: &Path, name: &str) -> Result<(), FsError> {
        let p = parent(path)?;
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios"
        ))]
        return fs::renameat_with(&p.fd, &p.name, &p.fd, name, fs::RenameFlags::NOREPLACE)
            .map_err(|e| error(path, e));
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios"
        )))]
        {
            let _ = (p, name);
            Err(FsError::Unsupported(
                "atomic rename without replacement".into(),
            ))
        }
    }
    pub(super) fn permissions(path: &Path, mode: u32) -> Result<(), FsError> {
        let p = parent(path)?;
        let s =
            fs::statat(&p.fd, &p.name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| error(path, e))?;
        if FileType::from_raw_mode(s.st_mode) == FileType::Symlink {
            return Err(FsError::Unsupported(
                "changing symbolic-link permissions".into(),
            ));
        }
        // Use libc's no-follow fchmodat wrapper. rustix's Linux backend
        // currently rejects this flag before reaching the kernel. The owned
        // parent descriptor stays alive throughout the call, including mode 000.
        use std::os::fd::AsRawFd;
        nix::sys::stat::fchmodat(
            Some(p.fd.as_raw_fd()),
            p.name.as_os_str(),
            nix::sys::stat::Mode::from_bits_truncate(mode),
            nix::sys::stat::FchmodatFlags::NoFollowSymlink,
        )
        .map_err(|e| error(path, e))
    }
    pub(super) fn remove(path: &Path, cancel: &AtomicBool) -> Result<(), FsError> {
        let p = parent(path)?;
        remove_entry(&p.fd, &p.name, cancel, 0, &mut 0).map_err(|e| error(path, e))
    }
    fn remove_entry(
        fd: &OwnedFd,
        name: &OsStr,
        cancel: &AtomicBool,
        depth: usize,
        count: &mut usize,
    ) -> Result<(), FsError> {
        cancelled(cancel)?;
        *count += 1;
        if depth > 256 || *count > 1_000_000 {
            return Err(FsError::Other(
                "Deletion limit reached; refresh to see remaining entries.".into(),
            ));
        }
        let s = fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| FsError::Other(e.to_string()))?;
        if FileType::from_raw_mode(s.st_mode) != FileType::Directory {
            return fs::unlinkat(fd, name, AtFlags::empty())
                .map_err(|e| FsError::Other(e.to_string()));
        }
        let child = fs::openat(
            fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| FsError::Other(e.to_string()))?;
        let mut entries = fs::Dir::read_from(&child).map_err(|e| FsError::Other(e.to_string()))?;
        for entry in &mut entries {
            let entry = entry.map_err(|e| FsError::Other(e.to_string()))?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                remove_entry(&child, OsStr::from_bytes(name), cancel, depth + 1, count)?;
            }
        }
        cancelled(cancel)?;
        fs::unlinkat(fd, name, AtFlags::REMOVEDIR).map_err(|e| FsError::Other(e.to_string()))
    }
}

// Platforms without directory-descriptor operations use bounded path traversal.
// Rechecks reject links already present at each step; they cannot eliminate an
// active ancestor replacement race between checking a path and using it.
#[cfg(any(not(unix), test))]
mod portable {
    use super::*;
    use std::path::{Component, PathBuf};

    pub(super) fn is_link(metadata: &std::fs::Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        }
        #[cfg(not(windows))]
        {
            metadata.is_symlink()
        }
    }

    pub(super) fn checked_path(path: &Path) -> Result<PathBuf, FsError> {
        if path.components().any(|c| c == Component::ParentDir) {
            return Err(error(
                path,
                "parent components are not accepted for mutations",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            if path.as_os_str().encode_wide().any(|c| c == 0) {
                return Err(error(path, "paths cannot contain NUL"));
            }
        }
        let absolute = std::path::absolute(path).map_err(|e| error(path, e))?;
        // Remove a trailing separator so the final symlink stays the final
        // component instead of being resolved by symlink_metadata.
        let normalized: PathBuf = absolute.components().collect();
        if normalized.file_name().is_none() {
            return Err(error(path, "the filesystem root cannot be changed"));
        }
        ancestors(&normalized)?;
        Ok(normalized)
    }

    fn ancestors(path: &Path) -> Result<(), FsError> {
        for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
            let metadata = std::fs::symlink_metadata(ancestor).map_err(|e| error(ancestor, e))?;
            if is_link(&metadata) || !metadata.is_dir() {
                return Err(error(
                    ancestor,
                    "mutation ancestors must be ordinary directories",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn remove(path: &Path, cancel: &AtomicBool) -> Result<(), FsError> {
        cancelled(cancel)?;
        let path = checked_path(path)?;
        remove_entry(&path, cancel, 0, &mut 0)
    }

    fn remove_entry(
        path: &Path,
        cancel: &AtomicBool,
        depth: usize,
        count: &mut usize,
    ) -> Result<(), FsError> {
        cancelled(cancel)?;
        *count += 1;
        if depth > 256 || *count > 1_000_000 {
            return Err(error(
                path,
                "deletion limit reached; refresh to see remaining entries",
            ));
        }
        ancestors(path)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|e| error(path, e))?;
        if metadata.is_dir() && !is_link(&metadata) {
            // Keep one iterator per ancestor, never collect a whole subtree.
            let mut entries = std::fs::read_dir(path).map_err(|e| error(path, e))?;
            loop {
                cancelled(cancel)?;
                let Some(entry) = entries.next() else { break };
                let child = entry.map_err(|e| error(path, e))?.path();
                remove_entry(&child, cancel, depth + 1, count)?;
            }
            cancelled(cancel)?;
            ancestors(path)?;
            // A directory swapped for a link while listing is rejected before
            // removal, including Windows junctions and other reparse points.
            let current = std::fs::symlink_metadata(path).map_err(|e| error(path, e))?;
            if is_link(&current) || !current.is_dir() {
                return Err(error(path, "directory changed during deletion"));
            }
            std::fs::remove_dir(path).map_err(|e| error(path, e))
        } else {
            cancelled(cancel)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
                // Junctions and directory symlinks are removed as directory
                // entries; their target contents are never traversed.
                if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    return std::fs::remove_dir(path).map_err(|e| error(path, e));
                }
            }
            std::fs::remove_file(path).map_err(|e| error(path, e))
        }
    }
}
pub(super) fn metadata(path: &Path) -> Result<FileMetadata, FsError> {
    #[cfg(unix)]
    {
        unix::metadata(path)
    }
    #[cfg(not(unix))]
    {
        let s = std::fs::symlink_metadata(path).map_err(|e| error(path, e))?;
        Ok(FileMetadata {
            kind: if s.is_dir() {
                EntryKind::Directory
            } else if s.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            },
            is_symlink: portable::is_link(&s),
            size: Some(s.len()),
            permissions: None,
            uid: None,
            gid: None,
            modified: s
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|t| t.as_secs()),
        })
    }
}
pub(super) fn rename(path: &Path, name: &str) -> Result<(), FsError> {
    if !valid_name(name) {
        return Err(error(
            path,
            "enter a single nonempty file name without separators",
        ));
    }
    #[cfg(unix)]
    {
        unix::rename(path, name)
    }
    #[cfg(windows)]
    {
        // Windows drive prefixes and alternate data streams contain ':'. A
        // rename may change only a single ordinary basename in this parent.
        let mut components = Path::new(name).components();
        if name.contains(':')
            || !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(error(
                path,
                "enter a single file name without a drive prefix or stream",
            ));
        }
        let path = portable::checked_path(path)?;
        let destination = path.with_file_name(name);
        // The safe dependency uses MoveFileExW without REPLACE_EXISTING, so a
        // destination created concurrently is preserved by the OS operation.
        atomicwrites::move_atomic(&path, &destination).map_err(|e| error(&path, e))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(FsError::Unsupported(
            "atomic rename without replacement".into(),
        ))
    }
}
pub(super) fn permissions(path: &Path, mode: u32) -> Result<(), FsError> {
    if mode & !0o7777 != 0 {
        return Err(error(path, "invalid permission mask"));
    }
    #[cfg(unix)]
    {
        unix::permissions(path, mode)
    }
    #[cfg(not(unix))]
    {
        Err(FsError::Unsupported(
            "POSIX permissions on this platform".into(),
        ))
    }
}
pub(super) fn remove(path: &Path, cancel: &AtomicBool) -> Result<(), FsError> {
    #[cfg(unix)]
    {
        unix::remove(path, cancel)
    }
    #[cfg(not(unix))]
    {
        portable::remove(path, cancel)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn rename_collision_permissions_and_link_deletion_preserve_the_target() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("file");
        std::fs::write(&file, b"original").unwrap();
        let exists = d.path().join("exists");
        std::fs::write(&exists, b"keep").unwrap();
        assert!(rename(&file, "exists").is_err());
        assert_eq!(std::fs::read(&exists).unwrap(), b"keep");
        rename(&file, "renamed").unwrap();
        let file = d.path().join("renamed");
        permissions(&file, 0o640).unwrap();
        assert_eq!(metadata(&file).unwrap().permissions, Some(0o640));
        permissions(&file, 0).unwrap();
        permissions(&file, 0o640).unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(permissions(&link, 0o777).is_err());
        remove(&link, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
    }
    #[test]
    fn recursive_delete_does_not_follow_links_and_cancel_preserves_entries() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"keep").unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(tree.join("nested")).unwrap();
        std::fs::write(tree.join("nested/file"), b"delete").unwrap();
        std::os::unix::fs::symlink(outside.path(), tree.join("link")).unwrap();
        assert!(remove(&tree, &AtomicBool::new(true)).is_err());
        assert!(tree.exists());
        remove(&tree, &AtomicBool::new(false)).unwrap();
        assert!(!tree.exists());
        assert!(outside.path().join("keep").exists());
        let alias = d.path().join("alias");
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        assert!(remove(&alias.join("keep"), &AtomicBool::new(false)).is_err());
        std::fs::write(d.path().join("victim"), b"keep").unwrap();
        assert!(remove(&alias.join("../victim"), &AtomicBool::new(false)).is_err());
        assert!(d.path().join("victim").exists());
        for invalid in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(!valid_name(invalid));
        }
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn portable_postorder_removes_files_and_empty_directories() {
        let d = tempfile::tempdir().unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir_all(tree.join("nested/empty")).unwrap();
        std::fs::write(tree.join("nested/file"), b"delete").unwrap();
        std::fs::write(d.path().join("keep"), b"keep").unwrap();
        portable::remove(&tree, &AtomicBool::new(false)).unwrap();
        assert!(!tree.exists());
        assert_eq!(std::fs::read(d.path().join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn portable_cancelled_delete_preserves_the_tree_and_rejects_root() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("keep"), b"keep").unwrap();
        assert!(portable::remove(d.path(), &AtomicBool::new(true)).is_err());
        assert_eq!(std::fs::read(d.path().join("keep")).unwrap(), b"keep");
        let absolute = std::path::absolute(d.path()).unwrap();
        let root = absolute.ancestors().last().unwrap();
        assert!(portable::remove(root, &AtomicBool::new(false)).is_err());
        assert!(portable::remove(&d.path().join("../keep"), &AtomicBool::new(false)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn portable_delete_unlinks_final_links_but_refuses_link_ancestors() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"keep").unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::os::unix::fs::symlink(outside.path(), tree.join("link")).unwrap();
        portable::remove(&tree, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
        let alias = d.path().join("alias");
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        assert!(portable::remove(&alias.join("keep"), &AtomicBool::new(false)).is_err());
        // A trailing separator must not turn an unlink into target traversal.
        portable::remove(&alias.join(""), &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn portable_cancellation_interrupts_a_started_traversal() {
        let d = tempfile::tempdir().unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        const FILES: usize = 2_000;
        for i in 0..FILES {
            std::fs::write(tree.join(i.to_string()), b"delete").unwrap();
        }
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| portable::remove(&tree, &cancel));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            // Wait for observable deletion before requesting cancellation.
            loop {
                let remaining = std::fs::read_dir(&tree).unwrap().count();
                if remaining < FILES {
                    assert!(remaining > 0);
                    cancel.store(true, Ordering::Release);
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(worker.join().unwrap().is_err());
        });
        assert!(tree.exists());
        let remaining = std::fs::read_dir(&tree).unwrap().count();
        assert!(remaining > 0 && remaining < FILES);
    }

    #[cfg(unix)]
    #[test]
    fn portable_depth_limit_preserves_the_unvisited_leaf() {
        let d = tempfile::tempdir().unwrap();
        let tree = d.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        let mut leaf = tree.clone();
        for _ in 0..257 {
            leaf.push("x");
            std::fs::create_dir(&leaf).unwrap();
        }
        std::fs::write(leaf.join("keep"), b"keep").unwrap();
        assert!(portable::remove(&tree, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(leaf.join("keep")).unwrap(), b"keep");
    }

    #[cfg(windows)]
    #[test]
    fn windows_rename_refuses_drive_prefixes_and_alternate_streams() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        std::fs::write(&source, b"original").unwrap();
        for name in ["C:new.txt", "C:", "source:stream", "source::$DATA"] {
            assert!(rename(&source, name).is_err(), "accepted {name}");
            assert_eq!(std::fs::read(&source).unwrap(), b"original");
        }
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn posix_remote_names_still_accept_colons() {
        assert!(valid_name("source:stream"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_rename_preserves_existing_file_and_directory() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(d.path().join("exists"), b"keep").unwrap();
        assert!(rename(&source, "exists").is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(std::fs::read(d.path().join("exists")).unwrap(), b"keep");
        std::fs::create_dir(d.path().join("directory")).unwrap();
        assert!(rename(&source, "directory").is_err());
        rename(&source, "renamed").unwrap();
        let folder = d.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        rename(&folder, "new-folder").unwrap();
        assert!(d.path().join("new-folder").is_dir());
    }
}
