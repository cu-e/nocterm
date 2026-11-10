//! Bounded command ownership; cancellation lives independently of read waiters.
use futures::FutureExt as _;
use nocterm_session::command_jobs::{
    CommandJobs, CommandSnapshot as Snapshot, CommandStart, CommandState as State, JobError,
};
use nocterm_session::{ExecRequest, HostExec};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const MAX_ACTIVE: usize = 4;
const MAX_RETAINED: usize = 16;
const MAX_STDOUT: usize = 64 * 1024;

fn append_output(snapshot: &mut Snapshot, bytes: &[u8]) {
    snapshot.total_bytes = snapshot.total_bytes.saturating_add(bytes.len() as u64);
    let keep = bytes
        .len()
        .min(MAX_STDOUT.saturating_sub(snapshot.stdout.len()));
    snapshot.stdout.extend_from_slice(&bytes[..keep]);
}

struct Job {
    terminal_id: String,
    executor: Arc<dyn HostExec>,
    snapshot: Arc<Mutex<Snapshot>>,
    cancel: async_channel::Sender<()>,
}
impl Job {
    fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn cancel(&self, state: State) {
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
pub(crate) struct Jobs {
    jobs: BTreeMap<u64, Job>,
    next: u64,
}
impl Jobs {
    fn start_owned(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<CommandStart, JobError> {
        let (id, state, cancel) = self.insert(terminal, executor.clone())?;
        Ok(CommandStart::new(
            id,
            Box::pin(run(executor, request, state, cancel, deadline)),
        ))
    }

    fn insert(
        &mut self,
        terminal_id: String,
        executor: Arc<dyn HostExec>,
    ) -> Result<(String, Arc<Mutex<Snapshot>>, async_channel::Receiver<()>), JobError> {
        if self
            .jobs
            .values()
            .filter(|job| job.snapshot().active())
            .count()
            >= MAX_ACTIVE
        {
            return Err(JobError::Busy);
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
    fn get(&self, id: &str, terminal: &str) -> Result<&Job, JobError> {
        id.strip_prefix('c')
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| self.jobs.get(&id))
            .filter(|job| job.terminal_id == terminal)
            .ok_or(JobError::Unknown)
    }
    fn cancel_terminal(&self, terminal: &str) {
        for job in self.jobs.values().filter(|job| job.terminal_id == terminal) {
            job.cancel(State::Cancelled);
        }
    }
    fn cancel_all(&self) {
        for job in self.jobs.values() {
            job.cancel(State::Cancelled);
        }
    }
    fn active(&self) -> impl Iterator<Item = (&str, &Arc<dyn HostExec>)> {
        self.jobs
            .values()
            .filter(|job| job.snapshot().active())
            .map(|job| (job.terminal_id.as_str(), &job.executor))
    }
}

/// Startup, stdin and continuous output draining all share one absolute deadline.
async fn run(
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
                    state.error = Some(error);
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
            append_output(
                &mut snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                &chunk,
            );
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

impl CommandJobs for Jobs {
    fn start(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: futures::future::BoxFuture<'static, ()>,
    ) -> Result<CommandStart, JobError> {
        self.start_owned(terminal, executor, request, deadline)
    }
    fn snapshot(&self, id: &str, terminal: &str) -> Result<Snapshot, JobError> {
        Ok(self.get(id, terminal)?.snapshot())
    }
    fn cancel(&self, id: &str, terminal: &str) -> Result<Snapshot, JobError> {
        let job = self.get(id, terminal)?;
        job.cancel(State::Cancelled);
        Ok(job.snapshot())
    }
    fn cancel_all(&self) {
        Jobs::cancel_all(self);
    }
    fn has_active(&self) -> bool {
        Jobs::active(self).next().is_some()
    }
    fn revoke_replaced(&self, current: &[(String, Option<Arc<dyn HostExec>>)]) {
        for (id, executor) in Jobs::active(self) {
            let replacement = current
                .iter()
                .find(|(candidate, _)| candidate == id)
                .and_then(|(_, executor)| executor.as_ref());
            if !replacement.is_some_and(|replacement| Arc::ptr_eq(replacement, executor)) {
                self.cancel_terminal(id);
            }
        }
    }
}
