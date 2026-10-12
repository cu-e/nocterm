use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_ai::AgentError;
use std::sync::Mutex;
#[cfg(unix)]
use std::time::Duration;

/// Why the agent process tree could not be confirmed stopped.
#[derive(Debug, Clone, thiserror::Error)]
pub(super) enum CleanupError {
    #[error("Agent cleanup worker disappeared")]
    WorkerLost,
    #[cfg(unix)]
    #[error("Cannot signal agent process group: {0}")]
    Signal(nix::errno::Errno),
    #[cfg(unix)]
    #[error("Cannot inspect agent process group: {0}")]
    Inspect(nix::errno::Errno),
    #[cfg(unix)]
    #[error("Cannot terminate agent process group: {0}")]
    Terminate(nix::errno::Errno),
    #[cfg(any(windows, test))]
    #[error("Cannot run taskkill to stop the agent process tree: {0}")]
    TaskkillFailed(std::sync::Arc<std::io::Error>),
    #[cfg(any(windows, test))]
    #[error("taskkill did not confirm agent process tree termination ({0})")]
    TaskkillUnconfirmed(std::process::ExitStatus),
    #[cfg(target_os = "linux")]
    #[error("Agent process container did not stop within six seconds")]
    ContainerTimeout,
}

type Completion = Shared<BoxFuture<'static, Result<(), CleanupError>>>;
pub(crate) struct ProcessGroup {
    pid: u32,
    #[cfg(target_os = "linux")]
    container: Option<std::sync::Arc<super::systemd::Container>>,
    completion: Mutex<Option<Completion>>,
}
impl ProcessGroup {
    pub(super) fn new(
        pid: u32,
        #[cfg(target_os = "linux")] container: Option<super::systemd::Container>,
    ) -> Self {
        Self {
            pid,
            #[cfg(target_os = "linux")]
            container: container.map(std::sync::Arc::new),
            completion: Mutex::new(None),
        }
    }
    fn cleanup(&self, force: bool) -> Completion {
        let mut completion = self
            .completion
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(completion) = completion.as_ref() {
            return completion.clone();
        }
        let pid = self.pid;
        #[cfg(target_os = "linux")]
        let container = self.container.clone();
        #[cfg(target_os = "linux")]
        if let Some(container) = &container {
            // The service leader can stop itself even if spawning a cleanup
            // worker fails under process or memory pressure.
            container.revoke();
        }
        let (tx, rx) = oneshot::channel();
        let future = async move { rx.await.unwrap_or(Err(CleanupError::WorkerLost)) }
            .boxed()
            .shared();
        *completion = Some(future.clone());
        let _worker = std::thread::Builder::new()
            .name("nocterm-agent-cleanup".into())
            .spawn(move || {
                #[cfg(target_os = "linux")]
                let result = if let Some(container) = container {
                    container.stop(force)
                } else {
                    stop_group(pid, force)
                };
                #[cfg(not(target_os = "linux"))]
                let result = stop_group(pid, force);
                let _ = tx.send(result);
            });
        future
    }
    pub(crate) fn shutdown(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        let completion = self.cleanup(false);
        async move {
            completion
                .await
                .map_err(|error| AgentError::Io(error.to_string()))
        }
        .boxed()
    }
    pub(crate) fn kill(&self) {
        drop(self.cleanup(true));
    }
    #[cfg(all(test, unix))]
    pub(crate) fn finish(&self) {
        let _ = futures::executor::block_on(self.shutdown());
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

fn stop_group(pid: u32, force: bool) -> Result<(), CleanupError> {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let pid = Pid::from_raw(pid as i32);
        let signal = killpg(
            pid,
            if force {
                Signal::SIGKILL
            } else {
                Signal::SIGTERM
            },
        );
        match signal {
            Ok(()) => {}
            Err(nix::errno::Errno::ESRCH) => return Ok(()),
            Err(error) => return Err(CleanupError::Signal(error)),
        }
        if !force {
            for _ in 0..20 {
                match killpg(pid, None) {
                    Err(nix::errno::Errno::ESRCH) => return Ok(()),
                    Err(error) => return Err(CleanupError::Inspect(error)),
                    Ok(()) => {}
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            match killpg(pid, Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => return Err(CleanupError::Terminate(error)),
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = force;
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        confirm_taskkill(status)?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn confirm_taskkill(result: std::io::Result<std::process::ExitStatus>) -> Result<(), CleanupError> {
    let status =
        result.map_err(|error| CleanupError::TaskkillFailed(std::sync::Arc::new(error)))?;
    if status.success() {
        Ok(())
    } else {
        Err(CleanupError::TaskkillUnconfirmed(status))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::confirm_taskkill;
    use std::os::unix::process::ExitStatusExt as _;
    #[test]
    fn taskkill_errors_never_acknowledge_process_cleanup() {
        assert!(confirm_taskkill(Ok(std::process::ExitStatus::from_raw(0))).is_ok());
        assert_eq!(
            confirm_taskkill(Ok(std::process::ExitStatus::from_raw(256)))
                .unwrap_err()
                .to_string(),
            "taskkill did not confirm agent process tree termination (exit status: 1)"
        );
        assert_eq!(
            confirm_taskkill(Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "missing executable"
            )))
            .unwrap_err()
            .to_string(),
            "Cannot run taskkill to stop the agent process tree: missing executable"
        );
    }
}
