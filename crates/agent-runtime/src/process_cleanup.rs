//! One lazy native cleanup attempt shared by normal close and final capture.
use super::startup::CleanupError;
use gpui::BackgroundExecutor;
use nocterm_ai::{
    connection::{ManagedCommandsHandle, ManagedProcessCleanup, ManagedShutdownWaiter},
    history::source::SourceLifetime,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Clones share one paid cleanup decision; none of them allocates or starts work.
#[derive(Clone)]
pub(super) struct ProcessCleanup {
    inner: ManagedProcessCleanup,
    executor: BackgroundExecutor,
}
impl ProcessCleanup {
    pub(super) fn new(
        commands: ManagedCommandsHandle,
        source: SourceLifetime,
        executor: BackgroundExecutor,
    ) -> Result<Self, CleanupError> {
        Ok(Self {
            inner: ManagedProcessCleanup::new(commands, source)?,
            executor,
        })
    }
    pub(super) fn owns(&self, commands: &ManagedCommandsHandle) -> bool {
        self.inner.owns(commands)
    }
    pub(super) fn commands(&self) -> &ManagedCommandsHandle {
        self.inner.commands()
    }
    pub(super) fn join(&self) -> Result<Completion, CleanupError> {
        Ok(Completion::new(self.inner.join()?, self.executor.clone()))
    }
    /// A failed ordinary attempt retains its process owner for one final retry.
    /// Pending or successful attempts are joined, never invoked a second time.
    pub(super) fn capture(&self) -> Result<Completion, CleanupError> {
        Ok(Completion::new(
            self.inner.capture()?,
            self.executor.clone(),
        ))
    }
}
/// The first poll hands the already-paid driver to the executor; joining never starts it.
pub(super) struct Completion {
    waiter: ManagedShutdownWaiter,
    executor: Option<BackgroundExecutor>,
}
impl Completion {
    fn new(waiter: ManagedShutdownWaiter, executor: BackgroundExecutor) -> Self {
        Self {
            waiter,
            executor: Some(executor),
        }
    }
}
impl Future for Completion {
    type Output = Result<(), CleanupError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(executor) = this.executor.take()
            && let Some(driver) = this.waiter.take_driver()
        {
            executor.spawn(driver).detach();
        }
        Pin::new(&mut this.waiter)
            .poll(cx)
            .map(|result| result.map_err(CleanupError::Process))
    }
}
