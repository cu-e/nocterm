#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "haiku"
))]
use nix::unistd::Pid;

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "haiku"
))]
pub(super) struct Observer(Pid);

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "haiku"
))]
impl Observer {
    pub(super) fn new(pid: u32) -> nix::Result<Self> {
        Ok(Self(Pid::from_raw(
            i32::try_from(pid).map_err(|_| nix::errno::Errno::EINVAL)?,
        )))
    }

    pub(super) fn exited(&mut self) -> nix::Result<bool> {
        use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
        let status = waitid(
            Id::Pid(self.0),
            WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
        )?;
        Ok(!matches!(status, WaitStatus::StillAlive))
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
mod kqueue {
    use nix::{
        errno::Errno,
        sys::event::{EventFilter, EventFlag, FilterFlag, KEvent, Kqueue},
    };

    pub(crate) struct Observer {
        queue: Kqueue,
        event: KEvent,
        exited: bool,
    }

    impl Observer {
        pub(crate) fn new(pid: u32) -> nix::Result<Self> {
            let event = KEvent::new(
                pid as usize,
                EventFilter::EVFILT_PROC,
                EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
                FilterFlag::NOTE_EXIT,
                0,
                0,
            );
            let mut this = Self {
                queue: Kqueue::new()?,
                event,
                exited: false,
            };
            // An owned, unreaped child may have already exited before its
            // notification is registered. ESRCH therefore means exited.
            match this.queue.kevent(&[event], &mut [], Some(zero())) {
                Ok(_) => {}
                Err(Errno::ESRCH) => this.exited = true,
                Err(error) => return Err(error),
            }
            Ok(this)
        }

        pub(crate) fn exited(&mut self) -> nix::Result<bool> {
            if !self.exited {
                let mut events = [self.event];
                self.exited = self.queue.kevent(&[], &mut events, Some(zero()))? > 0;
            }
            Ok(self.exited)
        }
    }

    fn zero() -> nix::libc::timespec {
        nix::libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub(super) use kqueue::Observer;
