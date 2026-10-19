//! Borrowed paid facts enter the existing native subprocess and containment owners.
#[cfg(unix)]
use super::{
    arguments::Arguments,
    diagnostic_text,
    environment::Environment,
    environment_snapshot::{self, Snapshot},
};
use super::{budget::SourceBudget, redaction::DiagnosticRedactor};
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::{
    connection::{ManagedStartupHandle, ManagedWorkspaceHandle},
    model::json::deferred::JsonDecodeError,
    sandbox::ProtectedWorkdirError,
    workspace::WorkdirError,
};
#[cfg(unix)]
use nocterm_ai::{model::native::NativeOwned, sandbox::MountAccess};
use std::{fmt, path::Path, sync::Arc};
#[cfg(unix)]
use std::{io, path::PathBuf, process::Stdio};
#[derive(thiserror::Error)]
pub(super) enum Error {
    #[cfg(unix)]
    #[error(transparent)]
    Snapshot(#[from] environment_snapshot::Error),
    #[error(transparent)]
    Allocation(#[from] ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Native(#[from] JsonDecodeError),
    #[error(transparent)]
    Path(#[from] nocterm_ai::model::native::NativePathError),
    #[error(transparent)]
    Workspace(#[from] crate::workspace::managed::Cause),
    #[error(transparent)]
    Workdir(#[from] WorkdirError),
    #[error(transparent)]
    Protected(#[from] ProtectedWorkdirError),
    #[error(transparent)]
    Redaction(#[from] super::redaction::Error),
    #[cfg(unix)]
    #[error("{}",.0.text())]
    Command(Source<SourceBudget>),
    #[cfg(unix)]
    #[error(
        "Agent isolation is on, but bubblewrap (bwrap) was not found in PATH. Install bubblewrap or turn isolation off in AI settings."
    )]
    SandboxMissing,
    #[cfg(unix)]
    #[error("Agent isolation is only available on Linux. Turn it off in AI settings.")]
    SandboxUnsupported,
    #[cfg(unix)]
    #[error("Cannot execute bubblewrap at {}",.0.as_path().display())]
    SandboxExecutable(NativeOwned<PathBuf>),
    #[cfg(unix)]
    #[error("Cannot start agent: {0}")]
    Spawn(#[source] io::Error),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Containment(#[from] crate::process::systemd::PreparationError),
    #[cfg(not(unix))]
    #[error("managed native process launch is unavailable on this platform")]
    Platform,
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
pub(super) struct Spawned {
    child: async_process::Child,
    group: Arc<crate::process::ProcessGroup>,
    secrets: Buffer<Source<SourceBudget>, SourceBudget>,
}
impl Spawned {
    pub(super) fn group(&self) -> &Arc<crate::process::ProcessGroup> {
        &self.group
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        async_process::Child,
        Arc<crate::process::ProcessGroup>,
        Buffer<Source<SourceBudget>, SourceBudget>,
    ) {
        (self.child, self.group, self.secrets)
    }
}
pub(super) struct Preparation {
    budget: SourceBudget,
}
impl Preparation {
    pub(super) fn new(budget: SourceBudget) -> Self {
        Self { budget }
    }
    #[cfg(unix)]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn spawn(
        &self,
        startup: &ManagedStartupHandle,
        workspace: &ManagedWorkspaceHandle,
        helper: Option<&Path>,
        rules: &DiagnosticRedactor,
    ) -> Result<Spawned, Error> {
        let launch = startup.facts().launch();
        let workspace = workspace.workspace();
        let snapshot = Snapshot::current(self.budget.clone())?;
        let env = Environment::from_snapshot(snapshot.raw(), launch, &self.budget)?;
        drop(snapshot);
        let secrets = env.secrets(launch, &self.budget)?;
        let executable = self
            .resolve(launch.command(), &env, &workspace.root)?
            .ok_or_else(|| self.missing(launch.command(), &env, rules))?;
        let (executable, args) = match &workspace.sandbox {
            Some(policy) => {
                self.sandbox(policy, &workspace.root, executable, launch.args(), &env)?
            }
            None => {
                let mut args = Arguments::new(self.budget.clone());
                for arg in launch.args() {
                    args.text(arg)?;
                }
                (executable, args)
            }
        };
        #[cfg(target_os = "linux")]
        let managed = helper
            .map(|helper| {
                crate::process::systemd::prepare_fields(
                    startup.facts().resources(),
                    &workspace.root,
                    helper,
                    executable.as_path(),
                    args.iter(),
                    env.iter().map(|(key, _)| key),
                )
            })
            .transpose()?;
        #[cfg(not(target_os = "linux"))]
        let _ = helper;
        #[cfg(target_os = "linux")]
        let mut command = managed.as_ref().map_or_else(
            || std::process::Command::new(executable.as_path()),
            |managed| managed.command(),
        );
        #[cfg(not(target_os = "linux"))]
        let mut command = std::process::Command::new(executable.as_path());
        #[cfg(target_os = "linux")]
        if managed.is_none() {
            command.args(args.iter());
        }
        #[cfg(not(target_os = "linux"))]
        command.args(args.iter());
        command
            .env_clear()
            .envs(env.iter())
            .current_dir(&workspace.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
        let child = async_process::Command::from(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(Error::Spawn)?;
        let group = Arc::new(crate::process::ProcessGroup::new(
            child.id(),
            #[cfg(target_os = "linux")]
            managed,
        ));
        Ok(Spawned {
            child,
            group,
            secrets,
        })
    }
    #[cfg(not(unix))]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    pub(super) fn spawn(
        &self,
        _: &ManagedStartupHandle,
        _: &ManagedWorkspaceHandle,
        _: Option<&Path>,
        _: &DiagnosticRedactor,
    ) -> Result<Spawned, Error> {
        let _ = &self.budget;
        Err(Error::Platform)
    }
    #[cfg(unix)]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn missing(&self, command: &str, env: &Environment, rules: &DiagnosticRedactor) -> Error {
        let result = diagnostic_text::format(format_args!("Cannot find agent command '{command}'. Configure an absolute executable path; inherited PATH is {}",env.get("PATH").unwrap_or("<unset>")),&self.budget).map_err(Error::from).and_then(|message|Ok(Error::Command(rules.scrub(message,&[],&self.budget)?)));
        match result {
            Ok(error) | Err(error) => error,
        }
    }
    #[cfg(unix)]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn resolve(
        &self,
        command: &str,
        env: &Environment,
        workdir: &Path,
    ) -> Result<Option<NativeOwned<PathBuf>>, Error> {
        let path = Path::new(command);
        let paths = crate::workspace::paths::Paths::new(self.budget.clone());
        if path.is_absolute() || path.components().count() > 1 {
            let candidate = if path.is_absolute() {
                self.budget.context().copy_path(path)?
            } else {
                NativeOwned::join_path(self.budget.ledger(), workdir, path)?
            };
            return Ok(paths.executable(candidate.as_path())?);
        }
        let Some(value) = env.get("PATH") else {
            return Ok(None);
        };
        for directory in value
            .split(':')
            .filter(|value| !value.is_empty())
            .map(Path::new)
        {
            let root = if directory.is_absolute() {
                self.budget.context().copy_path(directory)?
            } else {
                NativeOwned::join_path(self.budget.ledger(), workdir, directory)?
            };
            let candidate = NativeOwned::join_path(self.budget.ledger(), root.as_path(), path)?;
            if let Some(path) = paths.executable(candidate.as_path())? {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
    #[cfg(unix)]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn sandbox(
        &self,
        policy: &nocterm_ai::sandbox::SandboxPolicy,
        workdir: &Path,
        executable: NativeOwned<PathBuf>,
        args: &[String],
        env: &Environment,
    ) -> Result<(NativeOwned<PathBuf>, Arguments), Error> {
        if !cfg!(target_os = "linux") {
            return Err(Error::SandboxUnsupported);
        }
        let paths =
            crate::workspace::paths::Paths::new(self.budget.clone()).resolve(policy, workdir)?;
        if paths.get().workdir.resolved != workdir {
            return Err(WorkdirError::ChangedSincePrepared.into());
        }
        let mut output = Arguments::new(self.budget.clone());
        for arg in [
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--unshare-pid",
            "--die-with-parent",
            "--new-session",
            "--share-net",
        ] {
            output.text(arg)?;
        }
        for (path, access) in policy.planned_mounts(paths.get())? {
            match access {
                MountAccess::Writable => {
                    output.text("--bind")?;
                    output.path(path)?;
                    output.path(path)?;
                }
                MountAccess::ReadOnly => {
                    output.text("--ro-bind")?;
                    output.path(path)?;
                    output.path(path)?;
                }
                MountAccess::MaskDirectory => {
                    output.text("--tmpfs")?;
                    output.path(path)?;
                }
                MountAccess::MaskFile => {
                    output.text("--ro-bind")?;
                    output.text("/dev/null")?;
                    output.path(path)?;
                }
            }
        }
        output.text("--chdir")?;
        output.path(workdir)?;
        output.text("--")?;
        output.path(executable.as_path())?;
        for arg in args {
            output.text(arg)?;
        }
        let bwrap = self.bwrap(env, workdir)?.ok_or(Error::SandboxMissing)?;
        if !crate::workspace::paths::Paths::new(self.budget.clone())
            .is_executable(bwrap.as_path())?
        {
            return Err(Error::SandboxExecutable(bwrap));
        }
        Ok((bwrap, output))
    }
    #[cfg(unix)]
    #[expect(
        clippy::result_large_err,
        reason = "failures carry ledger-paid diagnostics by value; a Box would be an unpaid allocation"
    )]
    fn bwrap(
        &self,
        env: &Environment,
        workdir: &Path,
    ) -> Result<Option<NativeOwned<PathBuf>>, Error> {
        let Some(value) = env.get("PATH") else {
            return Ok(None);
        };
        for directory in value
            .split(':')
            .filter(|value| !value.is_empty())
            .map(Path::new)
        {
            let root = if directory.is_absolute() {
                self.budget.context().copy_path(directory)?
            } else {
                NativeOwned::join_path(self.budget.ledger(), workdir, directory)?
            };
            let candidate =
                NativeOwned::join_path(self.budget.ledger(), root.as_path(), Path::new("bwrap"))?;
            if crate::workspace::paths::Paths::new(self.budget.clone())
                .existing_file(candidate.as_path())?
            {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}
