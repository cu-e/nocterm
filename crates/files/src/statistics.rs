//! Folder statistics for both Explorer halves: what may be counted, the
//! running count and the remote walk. The local walk is in `local.rs`.

use crate::IndexingSettings;
use nocterm_session::{EntryKind, RemoteFs, fs::path, local_fs::CancelFlag};
use parking_lot::Mutex;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

/// Deeper subtrees are not entered.
pub(crate) const MAX_SCAN_DEPTH: usize = 256;
/// How often a running walk publishes its count.
const PUBLISH_EVERY: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Statistics {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub complete: bool,
    pub inaccessible: u64,
    pub errors: Vec<String>,
    /// Folders not entered because their name is excluded.
    pub excluded: u64,
    /// Why the size was not counted at all.
    pub skipped: Option<Skip>,
}

pub(crate) use nocterm_session::local_fs::ScanSkip as Skip;
impl From<nocterm_session::local_fs::Statistics> for Statistics {
    fn from(value: nocterm_session::local_fs::Statistics) -> Self {
        Self {
            files: value.files,
            directories: value.directories,
            bytes: value.bytes,
            complete: value.complete,
            inaccessible: value.inaccessible,
            errors: value.errors.iter().map(ToString::to_string).collect(),
            excluded: value.excluded,
            skipped: value.skipped,
        }
    }
}
/// What a walk may enter, from the user's settings.
#[derive(Clone, Debug)]
pub(crate) struct Policy {
    excluded: Vec<String>,
    pub(crate) max_entries: usize,
}

impl Policy {
    pub(crate) fn remote(settings: &IndexingSettings) -> Self {
        Self {
            excluded: settings.excluded.clone(),
            max_entries: settings.max_remote_entries as usize,
        }
    }
}

/// Why counting `path` on this computer is skipped, if it is.
pub(crate) fn local_skip(
    path: &Path,
    home: Option<&Path>,
    settings: &IndexingSettings,
) -> Option<Skip> {
    nocterm_session::local_fs::local_skip(
        path,
        home,
        settings.local,
        settings.skip_home,
        settings.skip_root,
    )
}

/// Why counting `directory` on a server is skipped, if it is.
pub(crate) fn remote_skip(
    directory: &str,
    home: Option<&str>,
    settings: &IndexingSettings,
) -> Option<Skip> {
    nocterm_session::local_fs::remote_skip(
        directory,
        home,
        settings.remote,
        settings.skip_home,
        settings.skip_root,
    )
}

/// The statistics of a listing that is not walked: its own entries only.
pub(crate) fn shallow(kinds: impl Iterator<Item = EntryKind>, skipped: Skip) -> Statistics {
    nocterm_session::local_fs::Statistics::shallow(kinds, skipped).into()
}

/// A walk in progress: cancelled when replaced or dropped.
#[derive(Default)]
pub(crate) struct Counter {
    progress: Arc<Mutex<Statistics>>,
    cancel: CancelFlag,
    /// What the view shows; caught up with the walk by [`Counter::poll`].
    pub(crate) shown: Statistics,
}

impl Counter {
    /// Cancels the running walk and returns the handles of a new one.
    pub(crate) fn restart(&mut self) -> (CancelFlag, Arc<Mutex<Statistics>>) {
        self.stop();
        self.cancel = CancelFlag::default();
        self.progress = Arc::default();
        self.shown = Statistics::default();
        (self.cancel.clone(), self.progress.clone())
    }

    /// Shows `statistics` without walking.
    pub(crate) fn set(&mut self, statistics: Statistics) {
        self.restart();
        *self.progress.lock() = statistics.clone();
        self.shown = statistics;
    }

    pub(crate) fn stop(&self) {
        self.cancel.cancel();
    }

    /// Copies the walk's latest count; whether it changed.
    pub(crate) fn poll(&mut self) -> bool {
        let latest = self.progress.lock().clone();
        let changed = latest != self.shown;
        self.shown = latest;
        changed
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) fn record_error(result: &mut Statistics, error: String) {
    result.inaccessible += 1;
    if result.errors.len() < 12 {
        result.errors.push(error);
    }
}

/// Counts a server folder over SFTP: one listing per folder entered.
pub(crate) async fn scan_remote(
    fs: Arc<dyn RemoteFs>,
    root: String,
    policy: Policy,
    cancel: CancelFlag,
    progress: Arc<Mutex<Statistics>>,
) {
    use nocterm_session::{
        FileMetadata,
        local_fs::{ScanDecision, ScanIssue, ScanPolicy, StatisticsAccumulator},
    };
    let scan_policy = match ScanPolicy::new(
        policy
            .excluded
            .iter()
            .map(std::ffi::OsString::from)
            .collect(),
        policy.max_entries,
        MAX_SCAN_DEPTH,
    ) {
        Ok(policy) => policy,
        Err(error) => {
            let mut result = Statistics {
                complete: true,
                ..Default::default()
            };
            record_error(&mut result, error.to_string());
            *progress.lock() = result;
            return;
        }
    };
    let mut count = StatisticsAccumulator::new(scan_policy);
    let mut pending = vec![(root, 0usize)];
    let mut published = Instant::now();
    'walk: while let Some((directory, depth)) = pending.pop() {
        if cancel.is_cancelled() {
            return;
        }
        let entries = match fs.read_dir(&directory).await {
            Ok(entries) => entries,
            Err(source) => {
                count.record_issue(ScanIssue::Remote {
                    path: directory,
                    source,
                });
                continue;
            }
        };
        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            let metadata = FileMetadata {
                kind: entry.kind,
                is_symlink: entry.is_symlink,
                size: entry.size,
                permissions: None,
                uid: None,
                gid: None,
                modified: None,
            };
            match count.observe(
                &metadata,
                depth + 1,
                depth == 0,
                std::ffi::OsStr::new(&entry.name),
            ) {
                ScanDecision::Enter => {
                    pending.push((path::join(&directory, &entry.name), depth + 1))
                }
                ScanDecision::Skip => {}
                ScanDecision::Stop => break 'walk,
            }
        }
        if published.elapsed() >= PUBLISH_EVERY {
            *progress.lock() = count.snapshot().into();
            published = Instant::now();
        }
    }
    if !cancel.is_cancelled() {
        *progress.lock() = count.finish().into();
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

/// The footer line describing `statistics`.
pub(crate) fn summary(statistics: &Statistics) -> String {
    let mut line = format!(
        "{} files · {} folders",
        statistics.files, statistics.directories
    );
    match statistics.skipped {
        Some(skip) => {
            line.push_str(" · ");
            line.push_str(match skip {
                Skip::Disabled => "size not counted",
                Skip::Home => "home folder size not counted",
                Skip::Root => "file system root size not counted",
            });
        }
        None => {
            line.push_str(" · ");
            line.push_str(&bytes(statistics.bytes));
            if !statistics.complete {
                line.push_str(" · calculating…");
            }
            if statistics.excluded > 0 {
                line.push_str(&format!(" · {} excluded", statistics.excluded));
            }
            if statistics.inaccessible > 0 {
                line.push_str(" · incomplete");
            }
        }
    }
    line
}

#[cfg(test)]
mod tests;
