//! What kind of host a watch talks to.

use std::time::Duration;

use nocterm_session::{ExecError, ExecRequest, HostExec};

use crate::{MetricSet, Reading, linux, windows};

/// The largest answer a detection probe reads.
const MAX_PROBE_OUTPUT: usize = 4096;

/// A host family the monitor has a script for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    /// Own Linux cgroup CPU/memory only; CPU percent is relative to one CPU.
    LinuxContainer,
    Windows,
    /// Anything else, by the name the host gave.
    Unsupported(String),
}

impl Platform {
    /// The long-running request that watches `metrics` on this platform.
    pub fn watch_request(&self, metrics: MetricSet, interval: Duration) -> Option<ExecRequest> {
        match self {
            Self::Linux => Some(linux::request(metrics, interval)),
            Self::LinuxContainer => Some(crate::container::request(metrics, interval)),
            Self::Windows => Some(windows::request(metrics, interval)),
            Self::Unsupported(_) => None,
        }
    }

    pub(crate) fn parse(&self, frame: &[String]) -> Reading {
        match self {
            Self::Linux => linux::parse(frame),
            Self::LinuxContainer => crate::container::parse(frame),
            Self::Windows => windows::parse(frame),
            Self::Unsupported(_) => Reading::default(),
        }
    }
}

/// The platform this program runs on.
pub fn local_platform() -> Platform {
    if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unsupported(std::env::consts::OS.to_owned())
    }
}

/// Asks a host what it is: `uname` first, then PowerShell, which is the only
/// one of the two a stock Windows has.
pub async fn detect(exec: &dyn HostExec) -> Result<Platform, ExecError> {
    let uname = output(exec, ExecRequest::new("uname").arg("-s")).await?;
    let system = uname.trim();
    // Git for Windows and MSYS2 put a `uname` on PATH; the host is still Windows.
    let posix_on_windows = ["MINGW", "MSYS", "CYGWIN"]
        .iter()
        .any(|prefix| system.starts_with(prefix));
    match system {
        "Linux" => return Ok(Platform::Linux),
        _ if posix_on_windows => return Ok(Platform::Windows),
        "" => {}
        other => return Ok(Platform::Unsupported(other.to_owned())),
    }
    let answer = output(exec, windows::powershell("Write-Output windows")).await?;
    Ok(if answer.trim().eq_ignore_ascii_case("windows") {
        Platform::Windows
    } else {
        Platform::Unsupported("unknown".into())
    })
}

/// A short program's whole output. A program that cannot start is no answer.
async fn output(exec: &dyn HostExec, request: ExecRequest) -> Result<String, ExecError> {
    match exec.exec(request).await {
        Ok(output) => Ok(output.collect(MAX_PROBE_OUTPUT).await.text()),
        Err(ExecError::Failed(_) | ExecError::NotFound(_)) => Ok(String::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nocterm_session::testing::FakeExec;

    fn detect_with(answers: Vec<(&str, Result<&str, ExecError>)>) -> Result<Platform, ExecError> {
        let exec = FakeExec::new(
            answers
                .into_iter()
                .map(|(program, answer)| (program, answer.map(|text| (text, 0, ""))))
                .collect(),
        );
        futures::executor::block_on(detect(&exec))
    }

    #[test]
    fn hosts_are_told_apart() {
        assert_eq!(
            detect_with(vec![("uname", Ok("Linux\n"))]),
            Ok(Platform::Linux)
        );
        assert_eq!(
            detect_with(vec![("uname", Ok("Darwin\n"))]),
            Ok(Platform::Unsupported("Darwin".into()))
        );
        assert_eq!(
            detect_with(vec![("uname", Ok("MINGW64_NT-10.0-22631\n"))]),
            Ok(Platform::Windows)
        );
        assert_eq!(
            detect_with(vec![("uname", Ok("")), ("powershell", Ok("windows\r\n"))]),
            Ok(Platform::Windows)
        );
        assert_eq!(
            detect_with(vec![]),
            Ok(Platform::Unsupported("unknown".into()))
        );
        assert_eq!(
            detect_with(vec![("uname", Err(ExecError::Disconnected))]),
            Err(ExecError::Disconnected)
        );
    }
}
