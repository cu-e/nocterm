//! Literal argv construction, private workspace selection and finite child limits.
use super::shell::{BASHRC, ShellAsset, validate_terminal};
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
    #[error("the contained shell configuration is unsupported")]
    Shell,
    #[error("the bundled contained shell asset could not be created")]
    ShellFile(#[source] nocterm_core::fs::FileError),
    #[error("the bundled contained shell asset could not be written")]
    ShellIo(#[source] std::io::Error),
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
    shell: ShellAsset,
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
            shell: ShellAsset::new()?,
        })
    }
    pub(super) fn wrap(&self, request: ExecRequest) -> Result<ExecRequest, ContainmentError> {
        self.wrap_in(request, namespace_arguments(&self.workspace))
    }
    pub(super) fn probe(&self) -> Result<ExecRequest, ContainmentError> {
        let mut namespace = namespace_arguments(&self.workspace);
        namespace.pop();
        self.shell.mount(&mut namespace);
        namespace.push("--".into());
        self.wrap_in(ExecRequest::new("/bin/bash").args([
            "--noprofile", "--norc", "-c",
            "test -r /run/nocterm/bashrc && test ! -w /run/nocterm/bashrc && test \"$PWD\" = /workspace",
        ]), namespace)
    }
    pub(super) fn launch(
        &self,
        launch: nocterm_session::ShellLaunch,
        term: &str,
        size: nocterm_session::PtySize,
    ) -> Result<nocterm_session::ShellLaunch, ContainmentError> {
        validate_terminal(term, size)?;
        if launch.validate().is_err()
            || !launch.env.is_empty()
            || launch
                .cwd
                .as_deref()
                .is_some_and(|cwd| cwd != "/workspace" && cwd != self.workspace)
        {
            return Err(ContainmentError::Shell);
        }
        let mut program = ExecRequest::new(launch.program.unwrap_or_else(|| "/bin/bash".into()));
        program.args = launch.args;
        let mut namespace = namespace_arguments(&self.workspace);
        namespace.pop();
        namespace.extend(["--setenv".into(), "TERM".into(), term.into()]);
        if launch.integration {
            if !matches!(program.program.as_str(), "/bin/bash" | "/usr/bin/bash")
                || !program.args.is_empty()
            {
                return Err(ContainmentError::Shell);
            }
            self.shell.mount(&mut namespace);
            program.args = ["--noprofile", "--rcfile", BASHRC, "-i"]
                .map(str::to_owned)
                .into();
        }
        namespace.push("--".into());
        let wrapped = self.wrap_in(program, namespace)?;
        Ok(nocterm_session::ShellLaunch {
            program: Some(wrapped.program),
            args: wrapped.args,
            cwd: Some(self.workspace.clone()),
            integration: false,
            ..Default::default()
        })
    }
    fn wrap_in(
        &self,
        request: ExecRequest,
        mut args: Vec<String>,
    ) -> Result<ExecRequest, ContainmentError> {
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
