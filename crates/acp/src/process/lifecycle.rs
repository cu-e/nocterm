use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_ai::AgentError;
use std::sync::Mutex;
#[cfg(unix)]
use std::time::Duration;

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;
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
        let future = async move {
            rx.await
                .unwrap_or_else(|_| Err("Agent cleanup worker disappeared".into()))
        }
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
        async move { completion.await.map_err(AgentError::Io) }.boxed()
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

fn stop_group(pid: u32, force: bool) -> Result<(), String> {
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
            Err(error) => return Err(format!("Cannot signal agent process group: {error}")),
        }
        if !force {
            for _ in 0..20 {
                match killpg(pid, None) {
                    Err(nix::errno::Errno::ESRCH) => return Ok(()),
                    Err(error) => {
                        return Err(format!("Cannot inspect agent process group: {error}"));
                    }
                    Ok(()) => {}
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            match killpg(pid, Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => return Err(format!("Cannot terminate agent process group: {error}")),
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
fn confirm_taskkill(result: std::io::Result<std::process::ExitStatus>) -> Result<(), String> {
    let status = result
        .map_err(|error| format!("Cannot run taskkill to stop the agent process tree: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill did not confirm agent process tree termination ({status})"
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::confirm_taskkill;
    use std::os::unix::process::ExitStatusExt as _;
    #[test]
    fn taskkill_errors_never_acknowledge_process_cleanup() {
        assert!(confirm_taskkill(Ok(std::process::ExitStatus::from_raw(0))).is_ok());
        assert!(confirm_taskkill(Ok(std::process::ExitStatus::from_raw(256))).is_err());
        assert!(
            confirm_taskkill(Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "missing executable"
            )))
            .is_err()
        );
    }
}
