//! Workspace filesystem work belongs to the background startup task.
use nocterm_ai::{AgentError, sandbox::SandboxPolicy};
use std::path::{Path, PathBuf};
pub(super) fn workspace(
    candidate: &Path,
    private: bool,
    isolated: bool,
    private_dirs: &[PathBuf],
    shared_dirs: &[PathBuf],
) -> Result<(PathBuf, Option<SandboxPolicy>), AgentError> {
    if private {
        nocterm_core::paths::ensure_private_dir(candidate)
            .map_err(|error| AgentError::Io(error.to_string()))?;
    } else if !candidate.is_absolute() || !candidate.is_dir() {
        return Err(AgentError::Io(
            "Working directory must be an existing absolute directory.".into(),
        ));
    }
    let root =
        std::fs::canonicalize(candidate).map_err(|error| AgentError::Io(error.to_string()))?;
    let policy = isolated.then(|| {
        SandboxPolicy::new(
            &root,
            std::env::home_dir().as_deref(),
            private_dirs,
            shared_dirs,
        )
    });
    if let Some(policy) = &policy {
        policy.validate_workdir(&root).map_err(AgentError::Io)?;
    }
    Ok((root, policy))
}
