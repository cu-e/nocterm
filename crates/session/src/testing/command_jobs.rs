//! Scripted command observations for consumers of the command port.
//! This fake never calls HostExec or drains transport output. Tests explicitly publish outcomes.
use super::lock;
use crate::command_jobs::{
    CommandJobs, CommandJobsFactory, CommandSnapshot, CommandStart, CommandState, JobError,
};
use crate::{ExecExit, ExecRequest, HostExec};
use futures::{FutureExt as _, future::BoxFuture};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub struct ScriptedCommandJobsFactory {
    commands: Arc<Mutex<Vec<Observation>>>,
}
struct Observation {
    request: ExecRequest,
    executor: Arc<dyn HostExec>,
    snapshot: Arc<Mutex<CommandSnapshot>>,
    done: async_channel::Sender<()>,
}
impl ScriptedCommandJobsFactory {
    pub fn requests(&self) -> Vec<ExecRequest> {
        lock(&self.commands)
            .iter()
            .map(|command| command.request.clone())
            .collect()
    }
    pub fn executor(&self, index: usize) -> Arc<dyn HostExec> {
        lock(&self.commands)[index].executor.clone()
    }
    pub fn snapshot(&self, index: usize) -> CommandSnapshot {
        lock(&lock(&self.commands)[index].snapshot).clone()
    }
    pub fn finish(&self, index: usize, stdout: &[u8], status: u32, stderr: &str) {
        let commands = lock(&self.commands);
        let command = &commands[index];
        let mut snapshot = lock(&command.snapshot);
        if !snapshot.active() {
            return;
        }
        snapshot.state = CommandState::Exited;
        snapshot.stdout = stdout.to_vec();
        snapshot.total_bytes = stdout.len() as u64;
        snapshot.exit = Some(ExecExit {
            status: Some(status),
            stderr: stderr.into(),
        });
        command.done.close();
    }
}
impl CommandJobsFactory for ScriptedCommandJobsFactory {
    fn create(&self) -> Box<dyn CommandJobs> {
        Box::new(ScriptedCommandJobs {
            script: self.clone(),
            entries: BTreeMap::new(),
            next: 0,
        })
    }
}
#[derive(Default)]
pub struct ScriptedCommandJobs {
    script: ScriptedCommandJobsFactory,
    entries: BTreeMap<String, ScriptedJob>,
    next: u64,
}
struct ScriptedJob {
    terminal: String,
    executor: Arc<dyn HostExec>,
    snapshot: Arc<Mutex<CommandSnapshot>>,
    done: async_channel::Sender<()>,
}
impl ScriptedJob {
    fn cancel(&self) {
        let mut snapshot = lock(&self.snapshot);
        if snapshot.active() {
            snapshot.state = CommandState::Cancelled;
        }
        self.done.close();
    }
}
impl Drop for ScriptedCommandJobs {
    fn drop(&mut self) {
        self.cancel_all();
    }
}
impl CommandJobs for ScriptedCommandJobs {
    fn start(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: BoxFuture<'static, ()>,
    ) -> Result<CommandStart, JobError> {
        if self
            .entries
            .values()
            .filter(|job| lock(&job.snapshot).active())
            .count()
            >= 4
        {
            return Err(JobError::Busy);
        }
        self.next += 1;
        let id = format!("c{}", self.next);
        let snapshot = Arc::new(Mutex::new(CommandSnapshot::default()));
        let (done, wait) = async_channel::bounded(1);
        lock(&self.script.commands).push(Observation {
            request,
            executor: executor.clone(),
            snapshot: snapshot.clone(),
            done: done.clone(),
        });
        self.entries.insert(
            id.clone(),
            ScriptedJob {
                terminal,
                executor,
                snapshot: snapshot.clone(),
                done,
            },
        );
        Ok(CommandStart::new(
            id,
            async move {
                if !lock(&snapshot).active() {
                    return;
                }
                lock(&snapshot).state = CommandState::Running;
                let done = wait.recv().fuse();
                let deadline = deadline.fuse();
                futures::pin_mut!(done, deadline);
                futures::select_biased! {
                    _ = done => {},
                    () = deadline => {
                        let mut snapshot = lock(&snapshot);
                        if snapshot.active() { snapshot.state = CommandState::TimedOut; }
                    },
                }
            }
            .boxed(),
        ))
    }
    fn snapshot(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, JobError> {
        self.entries
            .get(id)
            .filter(|job| job.terminal == terminal)
            .map(|job| lock(&job.snapshot).clone())
            .ok_or(JobError::Unknown)
    }
    fn cancel(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, JobError> {
        let job = self
            .entries
            .get(id)
            .filter(|job| job.terminal == terminal)
            .ok_or(JobError::Unknown)?;
        job.cancel();
        Ok(lock(&job.snapshot).clone())
    }
    fn cancel_all(&self) {
        for job in self.entries.values() {
            job.cancel();
        }
    }
    fn has_active(&self) -> bool {
        self.entries
            .values()
            .any(|job| lock(&job.snapshot).active())
    }
    fn revoke_replaced(&self, current: &[(String, Option<Arc<dyn HostExec>>)]) {
        for job in self.entries.values() {
            let replacement = current
                .iter()
                .find(|(id, _)| *id == job.terminal)
                .and_then(|(_, executor)| executor.as_ref());
            if !replacement.is_some_and(|executor| Arc::ptr_eq(executor, &job.executor)) {
                job.cancel();
            }
        }
    }
}
