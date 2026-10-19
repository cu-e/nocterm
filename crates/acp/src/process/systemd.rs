use super::{host, lifecycle::CleanupError};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Why a `systemctl --user` invocation did not complete.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SystemctlError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("systemd user manager rejected operation")]
    Rejected,
    #[error("systemd user manager operation timed out")]
    TimedOut,
}

pub(crate) struct Container {
    pub unit: String,
    pub lease: PathBuf,
    args: Vec<OsString>,
}
impl Container {
    pub(crate) fn command(&self) -> Command {
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

/// The same containment engine accepts immutable prepared native launch facts.
pub(crate) fn prepare_fields<'a>(
    resources: &nocterm_ai::AgentResourceSettings,
    workdir: &Path,
    helper: &Path,
    executable: &Path,
    args: impl Iterator<Item = &'a str>,
    keys: impl Iterator<Item = &'a str> + Clone,
) -> Result<Container, PreparationError> {
    preparation::Preparation::new(resources, workdir).run(helper, executable, args, keys)
}
mod preparation;
pub(crate) use preparation::PreparationError;

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
