//! Native PTY readiness, with separate wakeups for owner close and leader exit.
//!
//! Linux pidfds let an idle terminal sleep without missing a shell that exits
//! while a detached descendant retains its slave. Older kernels and sandboxes
//! denying pidfds retain the bounded process-status fallback.
use crate::LaunchError;
use filedescriptor::FileDescriptor;
use nix::{
    errno::Errno,
    fcntl::OFlag,
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::pipe2,
};
use std::{
    io,
    os::fd::{AsFd as _, AsRawFd, OwnedFd},
};

pub(super) struct Readiness {
    pty: FileDescriptor,
    closing: OwnedFd,
    leader: Option<OwnedFd>,
}

/// The borrow keeps the native master alive through safe descriptor duplication.
struct Master<'a>(&'a dyn portable_pty::MasterPty);
impl AsRawFd for Master<'_> {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd().unwrap_or(-1)
    }
}

impl Readiness {
    pub(super) fn new(
        master: &dyn portable_pty::MasterPty,
    ) -> Result<(Self, OwnedFd), LaunchError> {
        let pty = FileDescriptor::dup(&Master(master))?;
        let (closing, wake) = pipe2(OFlag::O_CLOEXEC).map_err(io::Error::from)?;
        Ok((
            Self {
                pty,
                closing,
                leader: None,
            },
            wake,
        ))
    }

    pub(super) fn observe_exit(&mut self, pid: Option<u32>) -> io::Result<()> {
        use rustix::{
            io::Errno,
            process::{Pid, PidfdFlags, pidfd_open},
        };
        let Some(pid) = pid
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
        else {
            return Ok(());
        };
        self.leader = match pidfd_open(pid, PidfdFlags::empty()) {
            Ok(fd) => Some(fd),
            Err(Errno::NOSYS | Errno::PERM | Errno::ACCESS) => None,
            Err(error) => return Err(error.into()),
        };
        Ok(())
    }

    pub(super) fn wait(&self) -> io::Result<()> {
        let mut descriptors = vec![
            PollFd::new(self.pty.as_fd(), PollFlags::POLLIN),
            PollFd::new(self.closing.as_fd(), PollFlags::POLLIN),
        ];
        if let Some(leader) = &self.leader {
            descriptors.push(PollFd::new(leader.as_fd(), PollFlags::POLLIN));
        }
        let timeout = if self.leader.is_some() {
            PollTimeout::NONE
        } else {
            PollTimeout::from(10u16)
        };
        match poll(&mut descriptors, timeout) {
            Ok(_) | Err(Errno::EINTR) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
