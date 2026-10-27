//! Work on the adapter's threads whose result any executor can await.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::FutureExt as _;
use futures::channel::oneshot;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

/// The result of a task on the adapter's runtime; abandons the task when dropped.
///
/// Resolves to `None` when the runtime stopped before the task finished.
pub(crate) struct Spawned<T> {
    result: oneshot::Receiver<T>,
    task: AbortHandle,
}

/// Runs `work` on the runtime of `handle`.
pub(crate) fn spawn<T: Send + 'static>(
    handle: &Handle,
    work: impl Future<Output = T> + Send + 'static,
) -> Spawned<T> {
    let (sender, result) = oneshot::channel();
    let task = handle
        .spawn(async move {
            let _ = sender.send(work.await);
        })
        .abort_handle();
    Spawned { result, task }
}

impl<T> Future for Spawned<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.result.poll_unpin(cx).map(Result::ok)
    }
}

impl<T> Drop for Spawned<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}
