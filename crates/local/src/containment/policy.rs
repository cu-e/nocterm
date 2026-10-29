//! Literal argv construction, private workspace selection and finite child limits.
use nocterm_session::{ConnectionFailure, ConnectionFailureKind, ExecError, ExecRequest};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum ContainmentError {
    #[error("the contained tool workspace must be a private, separate native directory")]
    Workspace,
    #[error("the child resource limits or literal arguments are invalid")]
    Limits,
    #[error("required Linux tool containment is unavailable")]
    Unavailable,
    #[error("the native containment probe failed")]
    Probe(#[source] ExecError),
}
impl ContainmentError {
    pub(super) fn connection_failure(self) -> ConnectionFailure {
        ConnectionFailure::new(ConnectionFailureKind::Denied, self)
    }
}

/// Process address space, process count, descriptors, CPU seconds and file size.
/// These are finite child limits, not an account cgroup or a measured RSS claim.
#[derive(Clone, Copy)]
pub struct ContainmentLimits {
    memory: u64,
    processes: u32,
    files: u32,
    cpu: u64,
    file_bytes: u64,
}
impl ContainmentLimits {
    pub fn new(
        memory: u64,
        processes: u32,
        files: u32,
        cpu: u64,
        file_bytes: u64,
    ) -> Result<Self, ContainmentError> {
        if !(64 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&memory)
            || !(8..=256).contains(&processes)
            || !(32..=512).contains(&files)
            || !(1..=3600).contains(&cpu)
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&file_bytes)
        {
            return Err(ContainmentError::Limits);
        }
        Ok(Self {
            memory,
            processes,
            files,
            cpu,
            file_bytes,
        })
    }
}
pub(super) struct ChildPolicy {
    workspace: String,
    limits: ContainmentLimits,
}
impl ChildPolicy {
    pub(super) fn new(
        workspace: PathBuf,
        state: PathBuf,
        limits: ContainmentLimits,
    ) -> Result<Self, ContainmentError> {
        private_directory(&workspace)?;
        private_directory(&state)?;
        if workspace.starts_with(&state) || state.starts_with(&workspace) {
            return Err(ContainmentError::Workspace);
        }
        if ["/usr", "/lib", "/lib64"]
            .iter()
            .any(|root| state.starts_with(root))
        {
            return Err(ContainmentError::Workspace);
        }
        let workspace = workspace.to_str().ok_or(ContainmentError::Workspace)?;
        if workspace.len() > 4096 {
            return Err(ContainmentError::Workspace);
        }
        Ok(Self {
            workspace: workspace.to_owned(),
            limits,
        })
    }
    pub(super) fn wrap(&self, request: ExecRequest) -> Result<ExecRequest, ContainmentError> {
        let argument_bytes = request
            .args
            .iter()
            .try_fold(request.program.len(), |n, argument| {
                n.checked_add(argument.len())
            });
        if request.program.is_empty()
            || request.program.contains('\0')
            || request.args.len() > 4096
            || request.args.iter().any(|argument| argument.contains('\0'))
            || argument_bytes.is_none_or(|bytes| bytes > 128 * 1024)
        {
            return Err(ContainmentError::Limits);
        }
        let limits = self.limits;
        let mut args = namespace_arguments(&self.workspace);
        // Apply the process limit inside the new user/PID namespace. Applying
        // it to the launcher instead counts unrelated processes of the host
        // uid and can prevent creating the isolation boundary itself.
        args.extend([
            "/usr/bin/prlimit".into(),
            format!("--as={0}:{0}", limits.memory),
            format!("--nproc={0}:{0}", limits.processes),
            format!("--nofile={0}:{0}", limits.files),
            format!("--cpu={0}:{0}", limits.cpu),
            format!("--fsize={0}:{0}", limits.file_bytes),
            "--".into(),
        ]);
        args.push(request.program);
        args.extend(request.args);
        Ok(ExecRequest {
            program: "/usr/bin/bwrap".into(),
            args,
            stdin: request.stdin,
        })
    }
}

fn namespace_arguments(workspace: &str) -> Vec<String> {
    [
        "--unshare-all",
        "--unshare-user",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/home/runner",
        "--setenv",
        "HOME",
        "/home/runner",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--bind",
        workspace,
        "/workspace",
        "--chdir",
        "/workspace",
        "--",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn private_directory(path: &Path) -> Result<(), ContainmentError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ContainmentError::Workspace);
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| ContainmentError::Workspace)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ContainmentError::Workspace);
        }
    }
    let metadata = fs::metadata(path).map_err(|_| ContainmentError::Workspace)?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(ContainmentError::Workspace);
    }
    Ok(())
}
