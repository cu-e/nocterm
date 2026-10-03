//! Local browsing and cancellable logical-byte statistics. No shell parsing.

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Statistics {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub complete: bool,
    pub inaccessible: u64,
    pub errors: Vec<String>,
}
pub(crate) fn home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_owned())
}
pub(crate) fn read_directory(path: &Path) -> Result<Vec<LocalEntry>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
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
        entries.push(LocalEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            kind,
            symlink,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}
pub(crate) fn scan(path: PathBuf, cancel: Arc<AtomicBool>, progress: Arc<Mutex<Statistics>>) {
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
    while let Some((directory, immediate)) = stack.last_mut() {
        if cancel.load(Ordering::Acquire) {
            return;
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
                match fs::read_dir(&path) {
                    Ok(dir) => stack.push((dir, false)),
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
fn record_error(result: &mut Statistics, error: String) {
    result.inaccessible += 1;
    if result.errors.len() < 12 {
        result.errors.push(error);
    }
}
pub(crate) fn bytes(value: u64) -> String {
    let (unit, scale) = if value >= 1 << 30 {
        ("GiB", (1u64 << 30) as f64)
    } else if value >= 1 << 20 {
        ("MiB", (1u64 << 20) as f64)
    } else if value >= 1 << 10 {
        ("KiB", (1u64 << 10) as f64)
    } else {
        return format!("{value} B");
    };
    format!("{:.1} {unit}", value as f64 / scale)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            Arc::new(AtomicBool::new(true)),
            progress.clone(),
        );
        assert!(!progress.lock().complete);
    }
}
