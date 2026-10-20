//! Structured jobs and their watchdog have one bounded owner; consumers receive snapshots only.
use super::super::execution::jobs;
use gpui::Task;
pub(in crate::thread) use jobs::Snapshot as CommandSnapshot;
use nocterm_ai::ToolFailure;
use nocterm_session::{ExecRequest, HostExec};
use std::sync::Arc;
#[derive(Default)]
pub(super) struct CommandJobs {
    jobs: jobs::Jobs,
    watch: Option<Task<()>>,
}
pub(in crate::thread) use jobs::CommandStart;
impl CommandJobs {
    pub(super) fn revoke_replaced(&self, current: &[(String, Option<Arc<dyn HostExec>>)]) {
        let revoked = self
            .jobs
            .active()
            .filter_map(|(id, executor)| {
                let replacement = current
                    .iter()
                    .find(|(candidate, _)| candidate == id)
                    .and_then(|(_, executor)| executor.as_ref());
                (!replacement.is_some_and(|replacement| Arc::ptr_eq(replacement, executor)))
                    .then_some(id.to_owned())
            })
            .collect::<Vec<_>>();
        for id in revoked {
            self.jobs.cancel_terminal(&id);
        }
    }

    pub(super) fn start(
        &mut self,
        terminal: String,
        executor: Arc<dyn HostExec>,
        request: ExecRequest,
        deadline: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<CommandStart, ToolFailure> {
        Ok(self.jobs.start(terminal, executor, request, deadline)?)
    }
    pub(super) fn snapshot(
        &self,
        id: &str,
        terminal: &str,
    ) -> Result<CommandSnapshot, ToolFailure> {
        Ok(self.jobs.get(id, terminal)?.snapshot())
    }
    pub(super) fn cancel(&self, id: &str, terminal: &str) -> Result<CommandSnapshot, ToolFailure> {
        let job = self.jobs.get(id, terminal)?;
        job.cancel(jobs::State::Cancelled);
        Ok(job.snapshot())
    }
    pub(super) fn active(&self) -> impl Iterator<Item = (&str, &Arc<dyn HostExec>)> {
        self.jobs.active()
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
        let mut jobs = CommandJobs::default();
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
            jobs::State::Cancelled
        );
        assert!(jobs.active().next().is_none());
        assert!(start(&mut jobs).is_ok());
        assert!(
            executor.requests().is_empty(),
            "only the owned runner performs process I/O"
        );
        drop(pending);
    }
}
