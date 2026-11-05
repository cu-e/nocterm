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

#[cfg(test)]
mod tests {
    use super::*;

    struct Release(Option<oneshot::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[test]
    fn a_result_can_be_awaited_by_an_executor_without_a_tokio_context() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        assert_eq!(
            futures::executor::block_on(spawn(runtime.handle(), async { 7 })),
            Some(7)
        );
    }

    #[tokio::test]
    async fn dropping_an_unpolled_answer_releases_the_task_resources() {
        let (sender, released) = oneshot::channel();
        let resource = Release(Some(sender));
        let answer = spawn(&Handle::current(), async move {
            let _resource = resource;
            std::future::pending::<()>().await;
        });
        drop(answer);
        tokio::time::timeout(std::time::Duration::from_secs(1), released)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn runtime_shutdown_ends_an_unanswered_task() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let answer = spawn(runtime.handle(), std::future::pending::<()>());
        runtime.shutdown_background();
        assert_eq!(futures::executor::block_on(answer), None);
    }
}
