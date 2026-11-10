//! Bounded agent commands without GUI state.
//!
//! Owns: per-chat command admission, continuous stdout draining, retained outcomes,
//! cancellation and absolute deadlines, retaining the original executor for every job.
//! Does not own: approvals, SSH authentication, model execution or views.
//! Ports used: session::HostExec. Ports provided:
//! session::command_jobs::{CommandJobs, CommandJobsFactory}.
//! Settings sections: none. Globals: none.
//! Invariants: at most four active and sixteen retained commands per registry;
//! stdout retains a 64KiB prefix while all bytes are drained and counted;
//! cancelling never invents a successful exit or replaces an accepted executor.
mod jobs;

#[derive(Default)]
pub struct JobsFactory;
impl nocterm_session::command_jobs::CommandJobsFactory for JobsFactory {
    fn create(&self) -> Box<dyn nocterm_session::command_jobs::CommandJobs> {
        Box::<jobs::Jobs>::default()
    }
}
