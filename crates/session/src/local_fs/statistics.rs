use super::{LocalFsError, ScanPolicy};
use crate::{EntryKind, FileMetadata, FsError};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanSkip {
    Disabled,
    Home,
    Root,
}

pub fn local_skip(
    path: &Path,
    home: Option<&Path>,
    enabled: bool,
    skip_home: bool,
    skip_root: bool,
) -> Option<ScanSkip> {
    if !enabled {
        Some(ScanSkip::Disabled)
    } else if skip_home && home.is_some_and(|home| home == path) {
        Some(ScanSkip::Home)
    } else if skip_root && path.parent().is_none() {
        Some(ScanSkip::Root)
    } else {
        None
    }
}

pub fn remote_skip(
    path: &str,
    home: Option<&str>,
    enabled: bool,
    skip_home: bool,
    skip_root: bool,
) -> Option<ScanSkip> {
    if !enabled {
        Some(ScanSkip::Disabled)
    } else if skip_root && path.trim_matches('/').is_empty() {
        Some(ScanSkip::Root)
    } else if skip_home
        && home.is_some_and(|home| home.trim_end_matches('/') == path.trim_end_matches('/'))
    {
        Some(ScanSkip::Home)
    } else {
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub complete: bool,
    pub inaccessible: u64,
    pub errors: Vec<ScanIssue>,
    pub excluded: u64,
    pub skipped: Option<ScanSkip>,
}
impl Statistics {
    pub fn shallow(kinds: impl Iterator<Item = EntryKind>, skipped: ScanSkip) -> Self {
        let mut result = Self {
            complete: true,
            skipped: Some(skipped),
            ..Self::default()
        };
        for kind in kinds {
            match kind {
                EntryKind::File => result.files = result.files.saturating_add(1),
                EntryKind::Directory => result.directories = result.directories.saturating_add(1),
                EntryKind::Other => {}
            }
        }
        result
    }
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum ScanIssue {
    #[error("{0}")]
    Local(#[from] LocalFsError),
    #[error("{path}: {source}", path = .path.display())]
    LocalPath {
        path: PathBuf,
        #[source]
        source: LocalFsError,
    },
    #[error("{path}: {source}")]
    Remote {
        path: String,
        #[source]
        source: FsError,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanDecision {
    Enter,
    Skip,
    Stop,
}

pub trait StatisticsObserver: Send + Sync {
    fn update(&self, statistics: Statistics);
}
impl<F: Fn(Statistics) + Send + Sync> StatisticsObserver for F {
    fn update(&self, statistics: Statistics) {
        self(statistics);
    }
}

pub struct StatisticsAccumulator {
    policy: ScanPolicy,
    result: Statistics,
    visited: usize,
}
impl StatisticsAccumulator {
    pub fn new(policy: ScanPolicy) -> Self {
        Self {
            policy,
            result: Statistics::default(),
            visited: 0,
        }
    }
    pub fn observe(
        &mut self,
        metadata: &FileMetadata,
        depth: usize,
        immediate: bool,
        name: &OsStr,
    ) -> ScanDecision {
        if !self.admit_entry() {
            return ScanDecision::Stop;
        }
        if metadata.is_symlink {
            return ScanDecision::Skip;
        }
        match metadata.kind {
            EntryKind::File => {
                self.result.bytes = self.result.bytes.saturating_add(metadata.size.unwrap_or(0));
                self.result.files = self.result.files.saturating_add(u64::from(immediate));
            }
            EntryKind::Directory => {
                self.result.directories =
                    self.result.directories.saturating_add(u64::from(immediate));
                if !self.policy.enters(name) {
                    self.result.excluded = self.result.excluded.saturating_add(1);
                } else if depth >= self.policy.depth() {
                    self.record_issue(ScanIssue::Local(LocalFsError::BoundExceeded(
                        "statistics depth",
                    )));
                } else {
                    return ScanDecision::Enter;
                }
            }
            EntryKind::Other => {}
        }
        ScanDecision::Skip
    }
    /// Failed metadata still represents an encountered child and consumes the walk budget.
    pub fn observe_entry_failure(&mut self, path: PathBuf, error: LocalFsError) -> ScanDecision {
        if !self.admit_entry() {
            return ScanDecision::Stop;
        }
        self.record_failure(path, error);
        ScanDecision::Skip
    }
    fn admit_entry(&mut self) -> bool {
        self.visited = self.visited.saturating_add(1);
        if self.visited > self.policy.entries() {
            self.record_issue(ScanIssue::Local(LocalFsError::BoundExceeded(
                "statistics entries",
            )));
            false
        } else {
            true
        }
    }
    pub fn record_failure(&mut self, path: PathBuf, error: LocalFsError) {
        self.record_issue(ScanIssue::LocalPath {
            path,
            source: error,
        });
    }
    pub fn record_issue(&mut self, issue: ScanIssue) {
        self.result.inaccessible = self.result.inaccessible.saturating_add(1);
        if self.result.errors.len() < 12 {
            self.result.errors.push(issue);
        }
    }
    pub fn snapshot(&self) -> Statistics {
        self.result.clone()
    }
    pub fn finish(mut self) -> Statistics {
        self.result.complete = true;
        self.result
    }
    pub fn cancelled(self) -> Statistics {
        self.result
    }
}
