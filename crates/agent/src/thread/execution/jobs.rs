//! Bounded command ownership; cancellation lives independently of read waiters.
use futures::FutureExt as _;
use nocterm_session::{ExecExit, ExecRequest, HostExec};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(super) const MAX_ACTIVE: usize = 4;
const MAX_RETAINED: usize = 16;
const MAX_STDOUT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Starting,
    Running,
    Exited,
    TimedOut,
    Cancelled,
    Failed,
}
impl State {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited => "exited",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
    fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub state: State,
    pub stdout: Vec<u8>,
    pub total_bytes: u64,
    pub exit: Option<ExecExit>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: State::Starting,
            stdout: Vec::new(),
            total_bytes: 0,
            exit: None,
            error: None,
        }
    }
}
impl Snapshot {
    pub(super) fn active(&self) -> bool {
        self.state.active()
    }
    fn append(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        let keep = bytes
            .len()
            .min(MAX_STDOUT.saturating_sub(self.stdout.len()));
        self.stdout.extend_from_slice(&bytes[..keep]);
    }
}
pub(super) struct Job {
    pub terminal_id: String,
    pub executor: Arc<dyn HostExec>,
    pub snapshot: Arc<Mutex<Snapshot>>,
    cancel: async_channel::Sender<()>,
}
impl Job {
    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub(super) fn cancel(&self, state: State) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if snapshot.active() {
            snapshot.state = state;
        }
        self.cancel.close();
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.close();
    }
}
#[derive(Default)]
pub(in crate::thread) struct Jobs {
    jobs: BTreeMap<u64, Job>,
    next: u64,
}
impl Jobs {
    pub(super) fn insert(
        &mut self,
        terminal_id: String,
        executor: Arc<dyn HostExec>,
    ) -> Result<(String, Arc<Mutex<Snapshot>>, async_channel::Receiver<()>), String> {
        if self
            .jobs
            .values()
            .filter(|job| job.snapshot().active())
            .count()
            >= MAX_ACTIVE
        {
            return Err(
                "This chat already has four active commands. Read or cancel one first.".into(),
            );
        }
        while self.jobs.len() >= MAX_RETAINED {
            let oldest = self
                .jobs
                .iter()
                .find(|(_, job)| !job.snapshot().active())
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.jobs.remove(&oldest);
            } else {
                break;
            }
        }
        self.next += 1;
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (cancel, receiver) = async_channel::bounded(1);
        self.jobs.insert(
            self.next,
            Job {
                terminal_id,
                executor,
                snapshot: snapshot.clone(),
                cancel,
            },
        );
        Ok((format!("c{}", self.next), snapshot, receiver))
    }
    pub(super) fn get(&self, id: &str, terminal: &str) -> Result<&Job, String> {
        id.strip_prefix('c')
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| self.jobs.get(&id))
            .filter(|job| job.terminal_id == terminal)
            .ok_or_else(|| "Unknown command for this chat and terminal.".into())
    }
    pub(super) fn cancel_terminal(&self, terminal: &str) {
        for job in self.jobs.values().filter(|job| job.terminal_id == terminal) {
            job.cancel(State::Cancelled);
        }
    }
    pub(super) fn cancel_all(&self) {
        for job in self.jobs.values() {
            job.cancel(State::Cancelled);
        }
    }
    pub(super) fn active(&self) -> impl Iterator<Item = (&str, &Arc<dyn HostExec>)> {
        self.jobs
            .values()
            .filter(|job| job.snapshot().active())
            .map(|job| (job.terminal_id.as_str(), &job.executor))
    }
}

/// Startup, stdin and continuous output draining all share one absolute deadline.
pub(super) async fn run(
    executor: Arc<dyn HostExec>,
    request: ExecRequest,
    snapshot: Arc<Mutex<Snapshot>>,
    cancel: async_channel::Receiver<()>,
    deadline: impl std::future::Future<Output = ()> + Send,
) {
    if !snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .active()
    {
        return;
    }
    let program = async {
        let mut output = match executor.exec(request).await {
            Ok(output) => output,
            Err(error) => {
                let mut state = snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.active() {
                    state.state = State::Failed;
                    state.error = Some(error.to_string());
                }
                return;
            }
        };
        {
            let mut state = snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.active() {
                return;
            }
            state.state = State::Running;
        }
        while let Some(chunk) = output.next().await {
            snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .append(&chunk);
        }
        let exit = output.exit().await;
        let mut state = snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active() {
            state.state = State::Exited;
            state.exit = Some(exit);
        }
    }
    .boxed();
    let stopped = async {
        let cancelled = cancel.recv().fuse();
        let deadline = deadline.fuse();
        futures::pin_mut!(cancelled, deadline);
        futures::select_biased! {
            _ = cancelled => State::Cancelled,
            () = deadline => State::TimedOut,
        }
    }
    .fuse();
    let program = program.fuse();
    futures::pin_mut!(stopped, program);
    let ended = futures::select_biased! {
        state = stopped => Some(state),
        () = program => None,
    };
    if let Some(ended) = ended {
        let mut state = snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active() {
            state.state = ended;
        }
    }
}
pub(super) fn timeout(value: Option<u64>) -> Duration {
    Duration::from_millis(value.unwrap_or(30_000))
}

#[cfg(test)]
mod tests;
