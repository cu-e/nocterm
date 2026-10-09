//! Owns the process tree until cleanup. Unix exit observation deliberately
//! leaves the leader unreaped, so its group ID cannot be reused before kill.

use nocterm_session::{ExecError, ExecRequest};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub(super) struct Process {
    #[cfg(unix)]
    child: Option<std::process::Child>,
    #[cfg(windows)]
    child: Option<Box<dyn process_wrap::std::ChildWrapper>>,
    #[cfg(unix)]
    observer: Option<unix::Observer>,
}

impl Process {
    pub(super) fn spawn(request: &ExecRequest) -> Result<Self, ExecError> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        let spawned = {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0).spawn()
        };
        #[cfg(windows)]
        let spawned = windows::spawn(command);
        let child = spawned.map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ExecError::NotFound(request.program.clone()),
            _ => ExecError::Failed(format!("{}: {error}", request.program)),
        })?;
        let this = Self {
            child: Some(child),
            #[cfg(unix)]
            observer: None,
        };
        #[cfg(unix)]
        let mut this = this;
        #[cfg(unix)]
        {
            // The guard already owns the child if observer setup fails.
            this.observer = Some(
                unix::Observer::new(this.child.as_ref().unwrap().id())
                    .map_err(|error| ExecError::Failed(error.to_string()))?,
            );
        }
        Ok(this)
    }

    pub(super) fn stdin(&mut self) -> Option<ChildStdin> {
        #[cfg(unix)]
        {
            self.child.as_mut()?.stdin.take()
        }
        #[cfg(windows)]
        {
            self.child.as_mut()?.stdin().take()
        }
    }

    pub(super) fn stdout(&mut self) -> Option<ChildStdout> {
        #[cfg(unix)]
        {
            self.child.as_mut()?.stdout.take()
        }
        #[cfg(windows)]
        {
            self.child.as_mut()?.stdout().take()
        }
    }

    pub(super) fn stderr(&mut self) -> Option<ChildStderr> {
        #[cfg(unix)]
        {
            self.child.as_mut()?.stderr.take()
        }
        #[cfg(windows)]
        {
            self.child.as_mut()?.stderr().take()
        }
    }

    pub(super) fn exited(&mut self) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            self.observer.as_mut().unwrap().exited().map_err(Into::into)
        }
        #[cfg(windows)]
        {
            Ok(self.child.as_mut().unwrap().try_wait()?.is_some())
        }
    }

    pub(super) fn stop(&mut self) -> Option<u32> {
        let mut child = self.child.take()?;
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            // No wait/try_wait happened before this call: even an exited
            // leader still owns its PID and protects this group's identity.
            if let Ok(pid) = i32::try_from(child.id()) {
                let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
            }
            let _ = child.kill();
        }
        #[cfg(windows)]
        let _ = child.start_kill();
        child
            .wait()
            .ok()?
            .code()
            .and_then(|code| u32::try_from(code).ok())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
pub(super) fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), ExecError> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let fd = pipe.as_raw_fd();
    let flags =
        fcntl(fd, FcntlArg::F_GETFL).map_err(|error| ExecError::Failed(error.to_string()))?;
    fcntl(
        fd,
        FcntlArg::F_SETFL(OFlag::from_bits_retain(flags) | OFlag::O_NONBLOCK),
    )
    .map(|_| ())
    .map_err(|error| ExecError::Failed(error.to_string()))
}
