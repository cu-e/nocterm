//! Deterministic in-memory implementations of session ports.
//!
//! Enable `test-support` in a consumer's dev-dependencies. These doubles perform no
//! filesystem, network or process I/O, spawn no workers and use no clock. Drive
//! [`SessionDriver`](crate::SessionDriver) and [`ExecSink`](crate::ExecSink) directly,
//! and use [`Gate`] to release pending operations from the test's executor.
mod command_jobs;
mod connection_source;
mod credentials;
pub use connection_source::ScriptedConnectionSource;
mod exec;
mod fs;
mod gate;
mod launch;
pub mod recording;
mod relay;
mod transport;

pub use command_jobs::{ScriptedCommandJobs, ScriptedCommandJobsFactory};
pub use credentials::{FakeCredentialError, FakeCredentialStore};
pub use exec::{ExecResponse, FakeExec};
pub use fs::{
    FsActivity, FsOperation, InMemoryFs, ListingReplyError, LocalRequestObserver, PendingListing,
};
pub use gate::Gate;
pub use launch::ScriptedLaunchPreparation;
pub use recording::{ScriptedOutputLog, ScriptedOutputLogFactory};
pub use relay::{RelayReply, RelayRequest, ScriptedRelay, capture_relay};
pub use transport::ScriptedTransport;

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
