//! Deterministic in-memory implementations of session ports.
//!
//! Enable `test-support` in a consumer's dev-dependencies. These doubles perform no
//! filesystem, network or process I/O, spawn no workers and use no clock. Drive
//! [`SessionDriver`](crate::SessionDriver) and [`ExecSink`](crate::ExecSink) directly,
//! and use [`Gate`] to release pending operations from the test's executor.
mod credentials;
mod exec;
mod fs;
mod gate;
pub mod recording;
mod transport;

pub use credentials::{FakeCredentialError, FakeCredentialStore};
pub use exec::{ExecResponse, FakeExec};
pub use fs::{
    FsActivity, FsOperation, InMemoryFs, ListingReplyError, LocalRequestObserver, PendingListing,
};
pub use gate::Gate;
pub use recording::{ScriptedOutputLog, ScriptedOutputLogFactory};
pub use transport::ScriptedTransport;

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
