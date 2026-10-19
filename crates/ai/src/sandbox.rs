//! Operating-system isolation of agent processes (Linux, bubblewrap).
//!
//! The policy follows the "workspace write" model of Codex and Claude Code's
//! sandbox: the whole file system is readable but not writable; the working
//! directory and the agent's own state and caches are writable; credential
//! stores are replaced by empty directories; the process tree gets its own
//! PID namespace and session, so it dies with nocterm and cannot inject input
//! into the controlling terminal. The network stays available: agents talk
//! to their model provider.
use std::path::{Path, PathBuf};

/// The requested workspace lies inside a protected credential store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("The agent workspace cannot be inside a protected credential store.")]
pub struct ProtectedWorkdirError;

/// Credential stores no agent needs, relative to the home directory.
const HIDDEN_IN_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gcloud",
    ".config/gh",
    ".config/doctl",
    ".password-store",
    ".local/share/keyrings",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".netrc",
    ".git-credentials",
    ".pgpass",
    ".vault-token",
    ".npmrc",
    ".pypirc",
];

/// Where agents and their package managers keep state, relative to the home
/// directory. Writable so that sign-in, history and `npx` downloads work.
const AGENT_STATE_IN_HOME: &[&str] = &[
    ".claude",
    ".claude.json",
    ".codex",
    ".hermes",
    ".gemini",
    ".npm",
    ".cache",
    ".local/state",
    ".config/claude",
];

/// What the sandbox shows of the file system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    /// Writable: the working directory and agent state.
    pub writable: Vec<PathBuf>,
    /// Replaced by empty directories or files.
    pub hidden: Vec<PathBuf>,
    /// Readable even though they lie under a hidden or temporary path, such
    /// as the directory of the terminal tools' socket.
    pub readable: Vec<PathBuf>,
    /// Credential roots cannot be used as a writable workspace.
    pub credentials: Vec<PathBuf>,
}

impl SandboxPolicy {
    /// The policy for an agent working in `workdir`. `private` lists
    /// nocterm's own directories (settings, vault, chats), hidden like
    /// credential stores; `shared` lists what must stay reachable inside them.
    pub fn new(
        workdir: &Path,
        home: Option<&Path>,
        private: &[PathBuf],
        shared: &[PathBuf],
    ) -> Self {
        let mut policy = Self {
            writable: vec![workdir.to_owned()],
            hidden: private.to_vec(),
            readable: shared.to_vec(),
            credentials: Vec::new(),
        };
        if let Some(home) = home {
            policy.credentials = HIDDEN_IN_HOME.iter().map(|path| home.join(path)).collect();
            policy.hidden.extend(policy.credentials.iter().cloned());
            policy
                .writable
                .extend(AGENT_STATE_IN_HOME.iter().map(|path| home.join(path)));
        }
        policy
    }

    /// Compare supplied lexical and physical identities without resolving any path.
    pub fn validate_workdir(
        &self,
        paths: &ResolvedSandboxPaths,
    ) -> Result<(), ProtectedWorkdirError> {
        if self
            .credentials
            .iter()
            .any(|root| paths.workdir.original.starts_with(root))
            || paths
                .credentials
                .iter()
                .any(|root| paths.workdir.resolved.starts_with(&root.resolved))
        {
            return Err(ProtectedWorkdirError);
        }
        Ok(())
    }
    /// Ordered policy effects; native adapters decide how to execute these mounts.
    pub fn mount_plan(
        &self,
        paths: &ResolvedSandboxPaths,
    ) -> Result<SandboxMountPlan, ProtectedWorkdirError> {
        self.validate_workdir(paths)?;
        let workdir = &paths.workdir.resolved;
        let mut mounts = Vec::new();
        for path in &paths.writable {
            if &path.resolved != workdir
                && path.kind != PathKind::Absent
                && !paths
                    .hidden
                    .iter()
                    .any(|root| path.resolved.starts_with(&root.resolved))
            {
                mounts.push(Mount::new(path.resolved.clone(), MountAccess::Writable));
            }
        }
        for path in paths
            .hidden
            .iter()
            .filter(|path| !path.resolved.starts_with(workdir))
        {
            mask(&mut mounts, path);
        }
        if paths.workdir.kind != PathKind::Absent {
            mounts.push(Mount::new(workdir.clone(), MountAccess::Writable));
        }
        for path in paths
            .hidden
            .iter()
            .filter(|path| path.resolved.starts_with(workdir))
        {
            mask(&mut mounts, path);
        }
        for path in &paths.readable {
            if path.kind != PathKind::Absent {
                mounts.push(Mount::new(path.resolved.clone(), MountAccess::ReadOnly));
            }
        }
        Ok(SandboxMountPlan(mounts))
    }
}
fn mask(mounts: &mut Vec<Mount>, path: &ResolvedPath) {
    let access = match path.kind {
        PathKind::Directory => MountAccess::MaskDirectory,
        PathKind::NonDirectory => MountAccess::MaskFile,
        PathKind::Absent => return,
    };
    mounts.push(Mount::new(path.resolved.clone(), access));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Absent,
    Directory,
    NonDirectory,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    pub original: PathBuf,
    pub resolved: PathBuf,
    pub kind: PathKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSandboxPaths {
    pub workdir: ResolvedPath,
    pub credentials: Vec<ResolvedPath>,
    pub writable: Vec<ResolvedPath>,
    pub hidden: Vec<ResolvedPath>,
    pub readable: Vec<ResolvedPath>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountAccess {
    Writable,
    ReadOnly,
    MaskDirectory,
    MaskFile,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub access: MountAccess,
}
impl Mount {
    fn new(path: PathBuf, access: MountAccess) -> Self {
        Self { path, access }
    }
}
pub struct SandboxMountPlan(Vec<Mount>);
impl SandboxMountPlan {
    pub fn mounts(&self) -> &[Mount] {
        &self.0
    }
}

/// Availability is display evidence, not authorization for a future launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available(PathBuf),
    Missing,
    Unsupported,
}
pub trait SandboxProbe: Send + Sync {
    fn availability(&self) -> Availability;
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum SandboxLaunchError {
    #[error(
        "Agent isolation is on, but bubblewrap (bwrap) was not found in PATH. Install bubblewrap or turn isolation off in AI settings."
    )]
    Missing,
    #[error("Agent isolation is only available on Linux. Turn it off in AI settings.")]
    Unsupported,
    #[error("Cannot execute bubblewrap at {0}")]
    NotExecutable(PathBuf),
    #[error("{0}")]
    Workspace(#[from] crate::workspace::WorkdirError),
    #[error("{0}")]
    Protected(#[from] ProtectedWorkdirError),
    #[error("Cannot start agent: {0}")]
    Spawn(#[source] nocterm_core::SharedError<std::io::Error>),
}
impl SandboxLaunchError {
    pub fn spawn(source: std::io::Error) -> Self {
        Self::Spawn(nocterm_core::SharedError::from_error(source))
    }
}
#[cfg(test)]
mod tests;
