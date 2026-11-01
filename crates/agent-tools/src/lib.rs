//! Bounded agent commands and run-scoped native terminal tools without GUI state.
//!
//! Owns: per-chat command admission, continuous stdout draining, retained outcomes,
//! cancellation and absolute deadlines, retaining the original executor for every job;
//! headless session receipts, physical Session/VT lifetimes, immutable run views
//! and eleven catalog handlers, including per-chat file transfers.
//! Does not own: account authentication, approvals, grant policy, SSH authentication,
//! journal execution, model execution or the supervising native task scheduler.
//! Ports used: session::HostExec, transfers::TransferRequests. Ports provided:
//! session::command_jobs::{CommandJobs, CommandJobsFactory},
//! cloud-model::ScopedToolExecutor. Headless consumers inject
//! current ToolAuthority, ToolTimer, ScopedSessionOpener, BackgroundCommandJournal
//! and supervised ToolTaskSpawner. Retained consumers additionally inject original
//! BackgroundCommandAuthority, BackgroundCommandCleanup and fresh ToolSessionHandleSource.
//! Settings sections: none. Globals: none.
//! Invariants: at most four active and sixteen retained commands per registry;
//! stdout retains a 64KiB prefix while all bytes are drained and counted;
//! cancelling never invents a successful exit or replaces an accepted executor.
//! Headless handles belong to one original run; session retirement and input/poll
//! admission serialize under the session entry. Geometry is at most 240x120 cells,
//! history 2000 rows and at most sixteen physical sessions per retained chat owner.
//! Retained run views have fresh handles and share the same job/session budget;
//! their detach/drop fences admission without closing physical connections. A
//! separate supervisor closes the retained owner; ordinary registries still close
//! on drop. One reader owns each Session. Live observations release only their
//! original private acquisition token. A background command
//! is linked to its durable original dispatch before native polling. Initial
//! candidates remain unavailable to tools/reattachment until explicit publication
//! after actual Connected and caller-owned durable establishment. One bounded
//! lifecycle receiver observes that physical generation without reading Session.
//! Existing-only factories never open a second connection. Their run-local
//! server aliases support idempotent open of the exact connected receipt. Initial
//! publication guards retire only their original untransferred physical owner.
//! Only an
//! observed exit acknowledged by that journal resolves its own effect; other
//! outcomes remain unknown. Restoring a screen never establishes a shell prompt.
//! Transfers: one chat-scoped book (at most four active, sixteen retained) is shared
//! by every run view of a retained chat. A run's authority names one session and
//! both transfer endpoints are checked independently against its receipt, so only
//! terminals of that session are admitted; any other terminal is a stale session.
//! Transfer effects are reported observed. Revoking a run or closing the chat
//! cancels the book; transfers do not survive a runner restart.
pub mod headless;
mod jobs;

#[derive(Default)]
pub struct JobsFactory;
impl nocterm_session::command_jobs::CommandJobsFactory for JobsFactory {
    fn create(&self) -> Box<dyn nocterm_session::command_jobs::CommandJobs> {
        Box::<jobs::Jobs>::default()
    }
}
