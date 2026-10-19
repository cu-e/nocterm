use super::{host, lifecycle::CleanupError};
use nocterm_ai::{AgentError, ConnectRequest};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    os::unix::fs::DirBuilderExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Why a `systemctl --user` invocation did not complete.
#[derive(Debug, thiserror::Error)]
enum SystemctlError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("systemd user manager rejected operation")]
    Rejected,
    #[error("systemd user manager operation timed out")]
    TimedOut,
}

pub(super) struct Container {
    pub unit: String,
    pub lease: PathBuf,
    args: Vec<OsString>,
}
impl Container {
    pub(super) fn command(&self) -> Command {
        let mut command = Command::new("/usr/bin/systemd-run");
        command.args(&self.args);
        command
    }
    pub(super) fn stop(&self, force: bool) -> Result<(), CleanupError> {
        self.revoke();
        let deadline = Instant::now() + Duration::from_secs(6);
        // Cancellation also reaches a helper whose StartTransientUnit call
        // was still in flight when the UI released its owner.
        loop {
            if force {
                let _ = control(&["kill", "--signal=KILL", "--kill-whom=all", &self.unit]);
            }
            let _ = control(&["stop", "--no-block", &self.unit]);
            let state = control(&["show", "--property=ActiveState", "--value", &self.unit]);
            if matches!(state.as_deref(), Ok("inactive" | "failed" | "")) {
                // A late helper cannot launch useful work after lease revocation.
                return Ok(());
            }
            if Instant::now() >= deadline {
                let _ = control(&["kill", "--signal=KILL", "--kill-whom=all", &self.unit]);
                return Err(CleanupError::ContainerTimeout);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    pub(super) fn revoke(&self) {
        let _ = fs::remove_dir(&self.lease);
    }
}
impl Drop for Container {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.lease);
    }
}

pub(super) fn prepare(
    request: &ConnectRequest,
    helper: &Path,
    executable: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Container, AgentError> {
    request
        .resources
        .validate()
        .map_err(|error| AgentError::Io(error.to_string()))?;
    control(&["show", "--property=Version", "--value"]).map_err(|_| AgentError::Io("Agent process containment requires a running systemd user manager and systemd-run. Start the user manager before sending a message.".into()))?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| AgentError::Io(error.to_string()))?;
    let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
    let unit = format!("nocterm-agent-{token}.service");
    let lease = std::env::temp_dir().join(format!("nocterm-agent-{token}"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&lease)
        .map_err(|error| AgentError::Io(error.to_string()))?;
    let mut container = Container {
        unit,
        lease,
        args: Vec::new(),
    };
    let owner = std::process::id();
    let identity = host::start_time(owner).map_err(|error| AgentError::Io(error.to_string()))?;
    let limits = &request.resources;
    let options = vec![
        "--user".to_owned(),
        "--quiet".into(),
        "--pipe".into(),
        "--wait".into(),
        "--collect".into(),
        "--service-type=exec".into(),
        "--expand-environment=no".into(),
        format!("--unit={}", container.unit),
        "--property=KillMode=control-group".into(),
        "--property=TimeoutStopSec=3s".into(),
        format!("--property=MemoryHigh={}M", limits.memory_high_mb),
        format!("--property=MemoryMax={}M", limits.memory_max_mb),
        format!("--property=MemorySwapMax={}M", limits.memory_swap_max_mb),
        format!("--property=TasksMax={}", limits.tasks_max),
        format!(
            "--working-directory={}",
            request.working_directory.display()
        ),
    ];
    container
        .args
        .extend(options.into_iter().map(OsString::from));
    container.args.extend(
        env.keys()
            .map(|key| OsString::from(format!("--setenv={key}"))),
    );
    container.args.extend([
        OsString::from("--"),
        helper.as_os_str().to_owned(),
        OsString::from("agent-host"),
        owner.to_string().into(),
        identity.to_string().into(),
        container.lease.as_os_str().to_owned(),
        env.keys().cloned().collect::<Vec<_>>().join(",").into(),
        executable.as_os_str().to_owned(),
    ]);
    container.args.extend(args.iter().map(OsString::from));
    Ok(container)
}

fn control(args: &[&str]) -> Result<String, SystemctlError> {
    async_io::block_on(async {
        use futures::{
            AsyncReadExt as _,
            future::{Either, select},
        };
        let mut child = async_process::Command::new("/usr/bin/systemctl")
            .arg("--user")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let operation = async {
            let mut output = String::new();
            stdout.read_to_string(&mut output).await?;
            let status = child.status().await?;
            if status.success() {
                Ok(output.trim().to_owned())
            } else {
                Err(SystemctlError::Rejected)
            }
        };
        match select(
            Box::pin(operation),
            Box::pin(async_io::Timer::after(Duration::from_secs(2))),
        )
        .await
        {
            Either::Left((result, _)) => result,
            Either::Right(_) => Err(SystemctlError::TimedOut),
        }
    })
}
