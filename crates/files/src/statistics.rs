//! Folder statistics for both Explorer halves: what may be counted, the
//! running count and the remote walk. The local walk is in `local.rs`.

use crate::IndexingSettings;
use nocterm_session::{EntryKind, RemoteFs, fs::path};
use parking_lot::Mutex;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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

/// Why a folder's size is not counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Skip {
    Disabled,
    Home,
    Root,
}

impl Skip {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Skip::Disabled => "size not counted",
            Skip::Home => "home folder size not counted",
            Skip::Root => "file system root size not counted",
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
    pub(crate) fn local(settings: &IndexingSettings) -> Self {
        Self {
            excluded: settings.excluded.clone(),
            max_entries: settings.max_local_entries as usize,
        }
    }

    pub(crate) fn remote(settings: &IndexingSettings) -> Self {
        Self {
            excluded: settings.excluded.clone(),
            max_entries: settings.max_remote_entries as usize,
        }
    }

    /// Whether a walk may enter a folder called `name`.
    pub(crate) fn enters(&self, name: &str) -> bool {
        !self.excluded.iter().any(|excluded| excluded == name)
    }

    pub(crate) fn limit_message(&self) -> String {
        format!(
            "Folder statistics stopped at the {} entry limit; the size is partial.",
            self.max_entries
        )
    }
}

/// Why counting `path` on this computer is skipped, if it is.
pub(crate) fn local_skip(
    path: &Path,
    home: Option<&Path>,
    settings: &IndexingSettings,
) -> Option<Skip> {
    if !settings.local {
        Some(Skip::Disabled)
    } else if settings.skip_home && home.is_some_and(|home| home == path) {
        Some(Skip::Home)
    } else if settings.skip_root && path.parent().is_none() {
        Some(Skip::Root)
    } else {
        None
    }
}

/// Why counting `directory` on a server is skipped, if it is.
pub(crate) fn remote_skip(
    directory: &str,
    home: Option<&str>,
    settings: &IndexingSettings,
) -> Option<Skip> {
    let trimmed = |path: &str| path.trim_end_matches('/').to_owned();
    if !settings.remote {
        Some(Skip::Disabled)
    } else if settings.skip_root && directory.trim_matches('/').is_empty() {
        Some(Skip::Root)
    } else if settings.skip_home && home.is_some_and(|home| trimmed(home) == trimmed(directory)) {
        Some(Skip::Home)
    } else {
        None
    }
}

/// The statistics of a listing that is not walked: its own entries only.
pub(crate) fn shallow(kinds: impl Iterator<Item = EntryKind>, skipped: Skip) -> Statistics {
    let mut statistics = Statistics {
        complete: true,
        skipped: Some(skipped),
        ..Statistics::default()
    };
    for kind in kinds {
        match kind {
            EntryKind::File => statistics.files += 1,
            EntryKind::Directory => statistics.directories += 1,
            EntryKind::Other => {}
        }
    }
    statistics
}

/// A walk in progress: cancelled when replaced or dropped.
pub(crate) struct Counter {
    progress: Arc<Mutex<Statistics>>,
    cancel: Arc<AtomicBool>,
    /// What the view shows; caught up with the walk by [`Counter::poll`].
    pub(crate) shown: Statistics,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            progress: Arc::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            shown: Statistics::default(),
        }
    }
}

impl Counter {
    /// Cancels the running walk and returns the handles of a new one.
    pub(crate) fn restart(&mut self) -> (Arc<AtomicBool>, Arc<Mutex<Statistics>>) {
        self.stop();
        self.cancel = Arc::new(AtomicBool::new(false));
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
        self.cancel.store(true, Ordering::Release);
    }

    /// Copies the walk's latest count; whether it changed.
    pub(crate) fn poll(&mut self) -> bool {
        let latest = self.progress.lock().clone();
        let changed = latest != self.shown;
        self.shown = latest;
        changed
    }

    #[cfg(test)]
    pub(crate) fn progress(&self) -> &Arc<Mutex<Statistics>> {
        &self.progress
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
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Statistics>>,
) {
    let mut result = Statistics::default();
    let mut pending = vec![(root, 0usize)];
    let mut visited = 0usize;
    let mut published = Instant::now();
    'walk: while let Some((directory, depth)) = pending.pop() {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let entries = match fs.read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) => {
                record_error(&mut result, format!("{directory}: {error}"));
                continue;
            }
        };
        for entry in entries {
            if entry.name == "." || entry.name == ".." || entry.is_symlink {
                continue;
            }
            visited += 1;
            if visited > policy.max_entries {
                record_error(&mut result, policy.limit_message());
                break 'walk;
            }
            let immediate = depth == 0;
            match entry.kind {
                EntryKind::File => {
                    result.bytes = result.bytes.saturating_add(entry.size.unwrap_or(0));
                    result.files += u64::from(immediate);
                }
                EntryKind::Directory => {
                    result.directories += u64::from(immediate);
                    if !policy.enters(&entry.name) {
                        result.excluded += 1;
                    } else if depth + 1 >= MAX_SCAN_DEPTH {
                        record_error(&mut result, "Folder statistics skipped a subtree deeper than 256 levels; the size is partial.".into());
                    } else {
                        pending.push((path::join(&directory, &entry.name), depth + 1));
                    }
                }
                EntryKind::Other => {}
            }
        }
        if published.elapsed() >= PUBLISH_EVERY {
            *progress.lock() = result.clone();
            published = Instant::now();
        }
    }
    if cancel.load(Ordering::Acquire) {
        return;
    }
    result.complete = true;
    *progress.lock() = result;
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
            line.push_str(skip.label());
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
