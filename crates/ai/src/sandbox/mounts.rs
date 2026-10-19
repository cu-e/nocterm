//! Borrowed policy effects use the same order for native and ordinary builders.
use super::{
    MountAccess, PathKind, ProtectedWorkdirError, ResolvedPath, ResolvedSandboxPaths, SandboxPolicy,
};
use std::path::Path;
pub struct PlannedMounts<'a> {
    paths: &'a ResolvedSandboxPaths,
    phase: u8,
    index: usize,
}
impl<'a> PlannedMounts<'a> {
    pub(super) fn new(
        policy: &SandboxPolicy,
        paths: &'a ResolvedSandboxPaths,
    ) -> Result<Self, ProtectedWorkdirError> {
        policy.validate_workdir(paths)?;
        Ok(Self {
            paths,
            phase: 0,
            index: 0,
        })
    }
    fn phase_item(&self, index: usize) -> Option<(&'a Path, MountAccess)> {
        let workdir = &self.paths.workdir.resolved;
        match self.phase {
            0 => self
                .paths
                .writable
                .get(index)
                .filter(|path| {
                    &path.resolved != workdir
                        && path.kind != PathKind::Absent
                        && !self
                            .paths
                            .hidden
                            .iter()
                            .any(|root| path.resolved.starts_with(&root.resolved))
                })
                .map(|path| (path.resolved.as_path(), MountAccess::Writable)),
            1 => self
                .paths
                .hidden
                .get(index)
                .filter(|path| !path.resolved.starts_with(workdir))
                .and_then(mask),
            2 => (self.paths.workdir.kind != PathKind::Absent)
                .then_some((workdir.as_path(), MountAccess::Writable)),
            3 => self
                .paths
                .hidden
                .get(index)
                .filter(|path| path.resolved.starts_with(workdir))
                .and_then(mask),
            4 => self
                .paths
                .readable
                .get(index)
                .filter(|path| path.kind != PathKind::Absent)
                .map(|path| (path.resolved.as_path(), MountAccess::ReadOnly)),
            _ => None,
        }
    }
    fn phase_len(&self) -> usize {
        match self.phase {
            0 => self.paths.writable.len(),
            1 | 3 => self.paths.hidden.len(),
            2 => 1,
            4 => self.paths.readable.len(),
            _ => 0,
        }
    }
}
fn mask(path: &ResolvedPath) -> Option<(&Path, MountAccess)> {
    let access = match path.kind {
        PathKind::Directory => MountAccess::MaskDirectory,
        PathKind::NonDirectory => MountAccess::MaskFile,
        PathKind::Absent => return None,
    };
    Some((path.resolved.as_path(), access))
}
impl<'a> Iterator for PlannedMounts<'a> {
    type Item = (&'a Path, MountAccess);
    fn next(&mut self) -> Option<Self::Item> {
        while self.phase < 5 {
            if self.index == self.phase_len() {
                self.phase += 1;
                self.index = 0;
                continue;
            }
            let index = self.index;
            self.index += 1;
            if let Some(item) = self.phase_item(index) {
                return Some(item);
            }
        }
        None
    }
}
