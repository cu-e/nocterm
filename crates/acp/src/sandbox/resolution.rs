//! One path resolution implementation shared by workspace preparation and launch.
use nocterm_ai::{
    sandbox::{PathKind, ResolvedPath, ResolvedSandboxPaths, SandboxPolicy},
    workspace::WorkdirError,
};
use std::path::Path;

pub(crate) struct PathResolver;
impl PathResolver {
    pub(crate) fn required_directory(path: &Path) -> Result<ResolvedPath, WorkdirError> {
        if !path.is_absolute() {
            return Err(WorkdirError::InvalidDirectory);
        }
        let facts = optional(path)?;
        if facts.kind != PathKind::Directory {
            return Err(WorkdirError::InvalidDirectory);
        }
        Ok(facts)
    }
    pub(crate) fn resolve(
        policy: &SandboxPolicy,
        workdir: &Path,
    ) -> Result<ResolvedSandboxPaths, WorkdirError> {
        Ok(ResolvedSandboxPaths {
            workdir: Self::required_directory(workdir)?,
            credentials: paths(&policy.credentials)?,
            writable: paths(&policy.writable)?,
            hidden: paths(&policy.hidden)?,
            readable: paths(&policy.readable)?,
        })
    }
}

fn optional(path: &Path) -> Result<ResolvedPath, WorkdirError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(ResolvedPath {
                original: path.to_owned(),
                resolved: path.to_owned(),
                kind: PathKind::Absent,
            });
        }
        Err(source) => return Err(error(path, source)),
    };
    let resolved = std::fs::canonicalize(path).map_err(|source| error(path, source))?;
    Ok(ResolvedPath {
        original: path.to_owned(),
        resolved,
        kind: if metadata.is_dir() {
            PathKind::Directory
        } else {
            PathKind::NonDirectory
        },
    })
}
fn paths(paths: &[std::path::PathBuf]) -> Result<Vec<ResolvedPath>, WorkdirError> {
    paths.iter().map(|path| optional(path)).collect()
}

fn error(path: &Path, source: std::io::Error) -> WorkdirError {
    WorkdirError::resolve(path.to_owned(), source)
}
