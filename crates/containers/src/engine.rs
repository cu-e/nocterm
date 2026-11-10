//! The engine's command line, and what running it means.

use futures::future::try_join;
use nocterm_session::{ExecOutput, ExecRequest, HostExec};

use crate::{
    Changes, ContainersError,
    model::{Container, Image, Snapshot},
    parse,
};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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

    /// The listings doing it changes.
    pub fn changes(self) -> Changes {
        match self {
            Self::RemoveImage => Changes::ALL,
            _ => Changes::CONTAINERS,
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
            Err(error) if matches!(failure, ContainersError::NotInstalled) => failure = error,
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

    pub fn command(self) -> ExecRequest {
        ExecRequest::new(match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        })
    }

    /// Read-only listing requests; execution belongs to the caller's adapter.
    pub fn container_listing(self) -> ExecRequest {
        self.command()
            .args(["ps", "--all", "--no-trunc", "--format", "json"])
    }

    pub fn image_listing(self) -> ExecRequest {
        self.command()
            .args(["images", "--all", "--no-trunc", "--format", "json"])
    }

    /// Every container, running or not, and every image.
    pub async fn snapshot(self, exec: &dyn HostExec) -> Result<Snapshot, ContainersError> {
        let (containers, images) = try_join(self.containers(exec), self.images(exec)).await?;
        Ok(Snapshot { containers, images })
    }

    /// Every container, running or not.
    pub async fn containers(self, exec: &dyn HostExec) -> Result<Vec<Container>, ContainersError> {
        let output = run(exec, self.listing("ps")).await?;
        parse::containers(&output).map_err(ContainersError::Parse)
    }

    /// Every image, intermediate ones included.
    pub async fn images(self, exec: &dyn HostExec) -> Result<Vec<Image>, ContainersError> {
        let output = run(exec, self.listing("images")).await?;
        parse::images(&output).map_err(ContainersError::Parse)
    }

    fn listing(self, what: &str) -> ExecRequest {
        self.command().args([what, "--all", "--format", "json"])
    }

    /// A stream of what happens to containers and images, one JSON object
    /// per line, until it is dropped; [`EventReader`](crate::EventReader)
    /// reads it.
    pub async fn events(self, exec: &dyn HostExec) -> Result<ExecOutput, ContainersError> {
        let request = self
            .command()
            .args(["events", "--filter", "type=container"])
            .args(["--filter", "type=image", "--format"])
            .arg(match self {
                // A template, which every Docker version reads.
                Self::Docker => "{{json .}}",
                Self::Podman => "json",
            });
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
        _ => Err(ContainersError::Exit {
            status: collected.exit.status,
            stderr_tail: bounded_stderr(collected.exit.stderr),
        }),
    }
}

/// A custom HostExec must not be able to retain unbounded stderr in an error.
fn bounded_stderr(mut stderr: String) -> String {
    const MAX_ERROR_TAIL: usize = 4 * 1024;
    let mut start = stderr.len().saturating_sub(MAX_ERROR_TAIL);
    while !stderr.is_char_boundary(start) {
        start += 1;
    }
    stderr.drain(..start);
    stderr
}

#[cfg(test)]
mod tests;
