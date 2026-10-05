//! Local browsing and cancellable logical-byte statistics. No shell parsing.

use crate::statistics::{MAX_SCAN_DEPTH, Policy, record_error};
pub(crate) use crate::statistics::{Statistics, bytes};
use nocterm_session::EntryKind;
use parking_lot::Mutex;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub(crate) struct LocalEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub symlink: bool,
}

pub(crate) fn home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_owned())
}
pub(crate) const MAX_DIRECTORY_ENTRIES: usize = 100_000;
pub(crate) const MAX_DIRECTORY_NAME_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn read_directory(path: &Path, cancel: &AtomicBool) -> Result<Vec<LocalEntry>, String> {
    let mut entries = Vec::new();
    let mut name_bytes = 0usize;
    for entry in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        if cancel.load(Ordering::Acquire) {
            return Err("Directory reading cancelled.".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        let symlink = metadata.is_symlink();
        let kind = if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        name_bytes = name_bytes.saturating_add(name.len());
        if entries.len() >= MAX_DIRECTORY_ENTRIES || name_bytes > MAX_DIRECTORY_NAME_BYTES {
            return Err(
                "Directory exceeds the Explorer listing limit; narrow the directory and refresh."
                    .into(),
            );
        }
        entries.push(LocalEntry {
            name,
            path: entry.path(),
            kind,
            symlink,
        });
    }
    entries.sort_by_cached_key(|entry| {
        (
            entry.kind != EntryKind::Directory,
            entry.name.to_lowercase(),
            entry.name.clone(),
        )
    });
    Ok(entries)
}
pub(crate) fn scan(
    path: PathBuf,
    policy: &Policy,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Statistics>>,
) {
    #[cfg(unix)]
    {
        scan_unix(path, policy, cancel, progress);
    }
    #[cfg(not(unix))]
    {
        scan_portable(path, policy, cancel, progress);
    }
}
#[cfg(not(unix))]
fn scan_portable(
    path: PathBuf,
    policy: &Policy,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Statistics>>,
) {
    let mut result = Statistics::default();
    let mut stack = match fs::read_dir(&path) {
        Ok(dir) => vec![(dir, true)],
        Err(e) => {
            result.inaccessible = 1;
            result.errors.push(format!("{}: {e}", path.display()));
            result.complete = true;
            *progress.lock() = result;
            return;
        }
    };
    let mut updated = Instant::now();
    let mut visited = 0usize;
    while let Some((directory, immediate)) = stack.last_mut() {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        visited += 1;
        if visited > policy.max_entries {
            record_error(&mut result, policy.limit_message());
            break;
        }
        let immediate = *immediate;
        let Some(entry) = directory.next() else {
            stack.pop();
            continue;
        };
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                record_error(&mut result, e.to_string());
                continue;
            }
        };
        let path = entry.path();
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_symlink() => {}
            Ok(metadata) if metadata.is_file() => {
                result.bytes = result.bytes.saturating_add(metadata.len());
                if immediate {
                    result.files += 1;
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                if immediate {
                    result.directories += 1;
                }
                if !entry
                    .file_name()
                    .to_str()
                    .is_none_or(|name| policy.enters(name))
                {
                    result.excluded += 1;
                    continue;
                }
                match fs::read_dir(&path) {
                    Ok(dir) if stack.len() < MAX_SCAN_DEPTH => stack.push((dir, false)),
                    Ok(_) => record_error(&mut result, "Folder statistics skipped a subtree deeper than 256 levels; the size is partial.".into()),
                    Err(e) => record_error(&mut result, format!("{}: {e}", path.display())),
                }
            }
            Ok(_) => {}
            Err(e) => record_error(&mut result, format!("{}: {e}", path.display())),
        }
        if updated.elapsed() >= Duration::from_millis(150) {
            *progress.lock() = result.clone();
            updated = Instant::now();
        }
    }
    result.complete = true;
    *progress.lock() = result;
}
#[cfg(unix)]
fn scan_unix(
    path: PathBuf,
    policy: &Policy,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Statistics>>,
) {
    use super::local_operations::unix::ScanDirectory;
    let mut result = Statistics::default();
    let root = match ScanDirectory::open(&path) {
        Ok(root) => root,
        Err(error) => {
            record_error(&mut result, error.to_string());
            result.complete = true;
            *progress.lock() = result;
            return;
        }
    };
    let mut stack = vec![(root, true)];
    let mut visited = 0usize;
    let mut updated = Instant::now();
    while let Some((directory, immediate)) = stack.last_mut() {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let immediate = *immediate;
        let Some(entry) = directory.next() else {
            stack.pop();
            continue;
        };
        visited += 1;
        if visited > policy.max_entries {
            record_error(&mut result, policy.limit_message());
            break;
        }
        let name = match entry {
            Ok(name) => name,
            Err(error) => {
                record_error(&mut result, error.to_string());
                continue;
            }
        };
        let metadata = match directory.metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) => {
                record_error(&mut result, error.to_string());
                continue;
            }
        };
        if metadata.is_symlink {
            continue;
        }
        match metadata.kind {
            EntryKind::File => {
                result.bytes = result.bytes.saturating_add(metadata.size.unwrap_or(0));
                if immediate {
                    result.files += 1;
                }
            }
            EntryKind::Directory => {
                if immediate {
                    result.directories += 1;
                }
                if !name.to_str().is_none_or(|name| policy.enters(name)) {
                    result.excluded += 1;
                    continue;
                }
                let child = directory.child(&name);
                if stack.len() >= MAX_SCAN_DEPTH {
                    record_error(&mut result, "Folder statistics skipped a subtree deeper than 256 levels; the size is partial.".into());
                } else {
                    match child {
                        Ok(child) => stack.push((child, false)),
                        Err(error) => record_error(&mut result, error.to_string()),
                    }
                }
            }
            EntryKind::Other => {}
        }
        if updated.elapsed() >= Duration::from_millis(150) {
            *progress.lock() = result.clone();
            updated = Instant::now();
        }
    }
    result.complete = true;
    *progress.lock() = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy::local(&nocterm_settings::IndexingSettings::default())
    }
    #[test]
    fn excluded_folders_are_counted_but_not_entered() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("node_modules/pkg/big"), [0u8; 4096]).unwrap();
        fs::write(dir.path().join("src/main.rs"), b"fn main(){}").unwrap();
        let progress = Arc::new(Mutex::new(Statistics::default()));
        scan(
            dir.path().into(),
            &policy(),
            Arc::new(AtomicBool::new(false)),
            progress.clone(),
        );
        let s = progress.lock();
        assert_eq!((s.directories, s.bytes, s.excluded), (2, 11, 1));
        assert!(s.complete);
    }
    #[test]
    fn counts_immediate_children_but_sizes_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("a"), b"123").unwrap();
        fs::write(dir.path().join("nested/b"), b"45678").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path(), dir.path().join("cycle")).unwrap();
        let progress = Arc::new(Mutex::new(Statistics::default()));
        scan(
            dir.path().into(),
            &policy(),
            Arc::new(AtomicBool::new(false)),
            progress.clone(),
        );
        let s = progress.lock();
        assert_eq!((s.files, s.directories, s.bytes), (1, 1, 8));
        assert!(s.complete);
    }
    #[test]
    fn cancelled_scan_does_not_publish_complete_stats() {
        let dir = tempfile::tempdir().unwrap();
        let progress = Arc::new(Mutex::new(Statistics::default()));
        scan(
            dir.path().into(),
            &policy(),
            Arc::new(AtomicBool::new(true)),
            progress.clone(),
        );
        assert!(!progress.lock().complete);
    }
    #[test]
    fn a_missing_directory_is_named_in_the_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("gone%");
        let error = read_directory(&missing, &AtomicBool::new(false)).unwrap_err();
        assert!(error.starts_with(&missing.display().to_string()), "{error}");
    }
    #[test]
    fn directory_read_obeys_cancellation_and_cached_case_order() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("z"), b"").unwrap();
        fs::write(root.path().join("a"), b"").unwrap();
        fs::create_dir(root.path().join("B")).unwrap();
        assert!(
            read_directory(root.path(), &AtomicBool::new(true))
                .unwrap_err()
                .contains("cancelled")
        );
        let entries = read_directory(root.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["B", "a", "z"]
        );
    }
    #[test]
    fn deeply_nested_statistics_are_explicitly_partial() {
        let root = tempfile::tempdir().unwrap();
        let mut leaf = root.path().to_owned();
        for _ in 0..260 {
            leaf = leaf.join("d");
            fs::create_dir(&leaf).unwrap();
        }
        fs::write(leaf.join("not-counted"), b"private").unwrap();
        let progress = Arc::new(Mutex::new(Statistics::default()));
        scan(
            root.path().into(),
            &policy(),
            Arc::new(AtomicBool::new(false)),
            progress.clone(),
        );
        let statistics = progress.lock();
        assert!(statistics.complete);
        assert!(statistics.inaccessible > 0);
        assert_eq!(statistics.bytes, 0);
        assert!(
            statistics
                .errors
                .iter()
                .any(|error| error.contains("deeper than 256"))
        );
    }
    #[cfg(unix)]
    #[test]
    fn held_statistics_directory_refuses_swapped_links_and_stays_on_original_inode() {
        use super::super::local_operations::unix::ScanDirectory;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let selected = root.path().join("selected");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(selected.join("child")).unwrap();
        fs::write(selected.join("original"), b"keep").unwrap();
        fs::write(outside.path().join("external"), b"secret").unwrap();
        let mut held = ScanDirectory::open(&selected).unwrap();
        fs::rename(&selected, root.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &selected).unwrap();
        let mut names = Vec::new();
        while let Some(entry) = held.next() {
            names.push(entry.unwrap());
        }
        assert!(names.iter().any(|name| name == "original"));
        assert!(!names.iter().any(|name| name == "external"));
        let original_child = root.path().join("moved/child");
        fs::remove_dir(&original_child).unwrap();
        std::os::unix::fs::symlink(outside.path(), &original_child).unwrap();
        assert!(held.child(std::ffi::OsStr::new("child")).is_err());
    }
}
