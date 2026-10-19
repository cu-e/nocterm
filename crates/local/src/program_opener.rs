//! Bounded native ownership of detached external editors.
use futures::channel::oneshot;
use nocterm_session::{ProgramOpenError, ProgramOpenFuture, ProgramOpenRequest, ProgramOpener};
use std::{
    io,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

/// Bound shared by all windows using the application's one opener instance.
pub const MAX_OPEN_PROGRAMS: usize = 32;

#[derive(Clone)]
pub struct LocalProgramOpener {
    slots: Arc<LaunchSlots>,
}
impl Default for LocalProgramOpener {
    fn default() -> Self {
        Self {
            slots: Arc::new(LaunchSlots::new(MAX_OPEN_PROGRAMS)),
        }
    }
}
impl ProgramOpener for LocalProgramOpener {
    fn open(&self, request: ProgramOpenRequest) -> ProgramOpenFuture {
        let slots = self.slots.clone();
        Box::pin(async move {
            let permit = slots.reserve()?;
            let (ready, started) = oneshot::channel();
            spawn_worker(request, permit, ready, |worker| {
                thread::Builder::new()
                    .name("nocterm-program-open".into())
                    .spawn(worker)
                    .map(|_| ())
            })
            .map_err(|source| ProgramOpenError::Worker { source })?;
            started
                .await
                .map_err(|_| ProgramOpenError::CompletionDropped)?
        })
    }
}

struct LaunchSlots {
    used: AtomicUsize,
    limit: usize,
}
impl LaunchSlots {
    fn new(limit: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
        }
    }
    fn reserve(self: &Arc<Self>) -> Result<LaunchPermit, ProgramOpenError> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < self.limit).then_some(used + 1)
            })
            .map_err(|_| ProgramOpenError::Busy { limit: self.limit })?;
        Ok(LaunchPermit {
            slots: Some(self.clone()),
        })
    }
}
struct LaunchPermit {
    slots: Option<Arc<LaunchSlots>>,
}
impl LaunchPermit {
    /// Exceptional wait failure consumes capacity permanently; it never proves reaping.
    fn retire(mut self) {
        self.slots.take();
    }
}
impl Drop for LaunchPermit {
    fn drop(&mut self) {
        if let Some(slots) = self.slots.take() {
            slots.used.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

type Worker = Box<dyn FnOnce() + Send>;
fn spawn_worker(
    request: ProgramOpenRequest,
    permit: LaunchPermit,
    ready: oneshot::Sender<Result<(), ProgramOpenError>>,
    spawn: impl FnOnce(Worker) -> io::Result<()>,
) -> io::Result<()> {
    spawn(Box::new(move || {
        let (program, args, cwd) = request.into_parts();
        let mut command = Command::new(&program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let started = command.spawn();
        drop(command);
        match started {
            Ok(child) => {
                let child = WaitingChild {
                    child: Some(child),
                    permit: Some(permit),
                };
                let _ = ready.send(Ok(()));
                child.wait();
            }
            Err(source) => {
                drop(permit);
                let _ = ready.send(Err(ProgramOpenError::Start { program, source }));
            }
        }
    }))
}

struct WaitingChild {
    child: Option<Child>,
    permit: Option<LaunchPermit>,
}
impl WaitingChild {
    fn wait(mut self) {
        self.reap();
    }
    fn reap(&mut self) {
        let Some(child) = &mut self.child else {
            return;
        };
        loop {
            match child.wait() {
                Ok(_) => {
                    self.child.take();
                    self.permit.take();
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    tracing::warn!(%error, "external program wait failed; retiring its capacity");
                    if let Some(permit) = self.permit.take() {
                        permit.retire();
                    }
                    self.child.take();
                    return;
                }
            }
        }
    }
}
impl Drop for WaitingChild {
    fn drop(&mut self) {
        self.reap();
    }
}

#[cfg(test)]
mod tests;
