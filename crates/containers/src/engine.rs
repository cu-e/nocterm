//! The engine's command line, and what running it means.

use std::fmt;

use futures::future::try_join;
use nocterm_session::{ExecError, ExecOutput, ExecRequest, HostExec};

use crate::{model::Snapshot, parse};

/// The most a listing may print; hosts with more containers than this
/// describes are not served.
const MAX_LISTING: usize = 16 * 1024 * 1024;
/// Lines of a container's log shown before following it.
const LOG_TAIL: &str = "1000";
/// The shell entered in a container: bash where there is one.
const SHELL: &str = "command -v bash >/dev/null 2>&1 && exec bash || exec sh";
/// How a POSIX shell reports a program it cannot find.
const NOT_FOUND: u32 = 127;

/// A container engine's command-line client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Docker,
    Podman,
}

/// Something done to containers, or to an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    /// Removes containers, stopping them first.
    Remove,
    RemoveImage,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::Remove | Self::RemoveImage => "Remove",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Start => &["start"],
            Self::Stop => &["stop"],
            Self::Restart => &["restart"],
            Self::Pause => &["pause"],
            Self::Unpause => &["unpause"],
            Self::Remove => &["rm", "--force"],
            Self::RemoveImage => &["rmi"],
        }
    }
}

/// Why the containers of a host are not known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainersError {
    /// Neither engine is installed on the host.
    NotInstalled,
    /// The host's session is not connected.
    Disconnected,
    /// The host cannot run programs.
    Unsupported,
    /// The engine failed, in its own words: its daemon is not running, the
    /// user may not use it, a container is gone.
    Failed(String),
}

impl fmt::Display for ContainersError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => formatter.write_str("Neither Docker nor Podman is installed."),
            Self::Disconnected => formatter.write_str("The session is not connected."),
            Self::Unsupported => formatter.write_str("This host cannot run programs."),
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ContainersError {}

impl From<ExecError> for ContainersError {
    fn from(error: ExecError) -> Self {
        match error {
            ExecError::NotFound(_) => Self::NotInstalled,
            ExecError::Disconnected => Self::Disconnected,
            ExecError::Unsupported => Self::Unsupported,
            ExecError::Failed(error) => Self::Failed(error),
        }
    }
}

/// The engine the host has, Docker first, with what it reports. When
/// neither answers, the error of the one that is installed, if any.
pub async fn detect(exec: &dyn HostExec) -> Result<(Engine, Snapshot), ContainersError> {
    let mut failure = ContainersError::NotInstalled;
    for engine in Engine::ALL {
        match engine.snapshot(exec).await {
            Ok(snapshot) => return Ok((engine, snapshot)),
            Err(ContainersError::NotInstalled) => {}
            Err(error @ (ContainersError::Disconnected | ContainersError::Unsupported)) => {
                return Err(error);
            }
            Err(error) if failure == ContainersError::NotInstalled => failure = error,
            Err(_) => {}
        }
    }
    Err(failure)
}

impl Engine {
    /// In the order they are looked for.
    pub const ALL: [Self; 2] = [Self::Docker, Self::Podman];

    pub fn name(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        }
    }

    fn command(self) -> ExecRequest {
        ExecRequest::new(match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        })
    }

    /// Every container, running or not, and every image.
    pub async fn snapshot(self, exec: &dyn HostExec) -> Result<Snapshot, ContainersError> {
        let list = |what: &str, all: &str| {
            let request = self
                .command()
                .arg(what)
                .arg(all)
                .arg("--format")
                .arg("json");
            run(exec, request)
        };
        let (containers, images) = try_join(list("ps", "--all"), list("images", "--all")).await?;
        Ok(Snapshot {
            containers: parse::containers(&containers).map_err(ContainersError::Failed)?,
            images: parse::images(&images).map_err(ContainersError::Failed)?,
        })
    }

    /// A stream that prints whenever a container or image changes, until it
    /// is dropped.
    pub async fn events(self, exec: &dyn HostExec) -> Result<ExecOutput, ContainersError> {
        let request = self
            .command()
            .args(["events", "--filter", "type=container"])
            .args(["--filter", "type=image"]);
        Ok(exec.exec(request).await?)
    }

    /// Does `action` to the containers or images `ids`.
    pub async fn perform(
        self,
        exec: &dyn HostExec,
        action: Action,
        ids: &[String],
    ) -> Result<(), ContainersError> {
        let request = self.command().args(action.args().iter().copied()).args(ids);
        run(exec, request).await.map(drop)
    }

    /// Follows a container's log, from its last lines; for a terminal.
    pub fn logs(self, id: &str) -> ExecRequest {
        self.command()
            .args(["logs", "--follow", "--tail", LOG_TAIL])
            .arg(id)
    }

    /// An interactive shell inside a running container; for a terminal.
    pub fn shell(self, id: &str) -> ExecRequest {
        self.command()
            .args(["exec", "--interactive", "--tty"])
            .arg(id)
            .args(["sh", "-c", SHELL])
    }
}

/// A short program's output, or what its failure means.
async fn run(exec: &dyn HostExec, request: ExecRequest) -> Result<String, ContainersError> {
    let collected = exec.exec(request).await?.collect(MAX_LISTING).await;
    match collected.exit.status {
        Some(0) => Ok(collected.text()),
        Some(NOT_FOUND) => Err(ContainersError::NotInstalled),
        _ => Err(ContainersError::Failed(collected.exit.error())),
    }
}

#[cfg(test)]
mod tests;
