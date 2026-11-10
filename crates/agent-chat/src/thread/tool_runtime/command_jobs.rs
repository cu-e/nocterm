//! Structured jobs and their watchdog have one bounded owner; consumers receive snapshots only.
use gpui::Task;
pub(in crate::thread) use jobs::CommandSnapshot;
use nocterm_ai::ToolFailure;
use nocterm_session::command_jobs as jobs;
use nocterm_session::{ExecRequest, HostExec};
use std::sync::Arc;
pub(super) struct CommandJobs {
    jobs: Box<dyn jobs::CommandJobs>,
    watch: Option<Task<()>>,
}
pub(in crate::thread) use jobs::CommandStart;
impl CommandJobs {
    pub(super) fn new(jobs: Box<dyn jobs::CommandJobs>) -> Self {
        Self { jobs, watch: None }
    }
    pub(super) fn revoke_replaced(&self, current: &[(String, Option<Arc<dyn HostExec>>)]) {
        self.jobs.revoke_replaced(current);
    }

    pub(super) fn start(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<CommandStart, ToolFailure> {
        self.jobs
            .start(terminal, executor, request, Box::pin(deadline))
            .map_err(|error| error.to_string().into())
    }
    pub(super) fn snapshot(
        &self,
        id: &str,
        terminal: &str,
    ) -> Result<CommandSnapshot, ToolFailure> {
        self.jobs
            .snapshot(id, terminal)
            .map_err(|error| error.to_string().into())
    }
    pub(super) fn cancel(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, ToolFailure> {
        self.jobs
            .cancel(id, terminal)
            .map_err(|error| error.to_string().into())
    }
    pub(super) fn has_active(&self) -> bool {
        self.jobs.has_active()
    }

    pub(super) fn cancel_all(&mut self) {
        self.watch = None;
        self.jobs.cancel_all();
    }
    pub(super) fn has_watch(&self) -> bool {
        self.watch.is_some()
    }
    pub(super) fn install_watch(&mut self, watch: Task<()>) {
        self.watch = Some(watch);
    }
    pub(super) fn finish_watch(&mut self) {
        self.watch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_bounds_original_jobs_and_cancellation_retains_readable_snapshots() {
        let executor = Arc::new(nocterm_session::testing::FakeExec::default());
        let mut jobs =
            CommandJobs::new(Box::<nocterm_session::testing::ScriptedCommandJobs>::default());
        let start = |jobs: &mut CommandJobs| {
            jobs.start(
                "terminal".into(),
                executor.clone(),
                ExecRequest {
                    program: "program".into(),
                    args: Vec::new(),
                    stdin: None,
                },
                futures::future::pending(),
            )
        };
        let first = start(&mut jobs).unwrap();
        let (id, first_run) = first.into_run();
        let mut pending = vec![first_run];
        for _ in 0..3 {
            pending.push(start(&mut jobs).unwrap().into_run().1);
        }
        assert!(start(&mut jobs).is_err());
        assert!(jobs.snapshot(&id, "terminal").unwrap().active());
        assert!(jobs.snapshot(&id, "replacement terminal").is_err());
        jobs.cancel_all();
        assert_eq!(
            jobs.snapshot(&id, "terminal").unwrap().state(),
            jobs::CommandState::Cancelled
        );
        assert!(!jobs.has_active());
        assert!(start(&mut jobs).is_ok());
        assert!(
            executor.requests().is_empty(),
            "only the owned runner performs process I/O"
        );
        drop(pending);
    }
}
