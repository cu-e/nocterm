//! Finite native file work for one Cloud device installation.
use std::sync::Arc;
use tokio::sync::Semaphore;

/// A native operation could not be admitted or completed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StorageWorkError {
    #[error("Cloud file work is busy")]
    Busy,
    #[error("the Cloud file worker is unavailable")]
    Unavailable,
    #[error("the Cloud file worker failed")]
    Join(#[source] tokio::task::JoinError),
}

/// Callers admit at most eight operations before waiting for two physical workers.
/// A started closure retains both permits through actual completion, even if its
/// async caller goes away.
pub(crate) struct StorageWork {
    physical: Arc<Semaphore>,
    operations: Arc<Semaphore>,
}
impl StorageWork {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_limits(2, 8)
    }
    fn with_limits(physical: usize, operations: usize) -> Arc<Self> {
        Arc::new(Self {
            physical: Arc::new(Semaphore::new(physical)),
            operations: Arc::new(Semaphore::new(operations)),
        })
    }
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, StorageWorkError> {
        let admitted = self
            .operations
            .clone()
            .try_acquire_owned()
            .map_err(|_| StorageWorkError::Busy)?;
        let physical = self
            .physical
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageWorkError::Unavailable)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| StorageWorkError::Unavailable)?;
        runtime
            .spawn_blocking(move || {
                let (_admitted, _physical) = (admitted, physical);
                operation()
            })
            .await
            .map_err(StorageWorkError::Join)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_session::testing::Gate;

    #[tokio::test]
    async fn a_cancelled_caller_keeps_its_native_work_counted_until_it_finishes() {
        let work = StorageWork::with_limits(2, 2);
        let release = Gate::pending();
        let (started, mut seen) = tokio::sync::mpsc::unbounded_channel();
        let mut callers = Vec::new();
        for _ in 0..2 {
            let (work, release, started) = (work.clone(), release.clone(), started.clone());
            callers.push(tokio::spawn(async move {
                work.run(move || {
                    started.send(()).unwrap();
                    futures::executor::block_on(release.wait());
                })
                .await
            }));
        }
        seen.recv().await.unwrap();
        seen.recv().await.unwrap();
        let cancelled = callers.remove(0);
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        assert!(matches!(work.run(|| ()).await, Err(StorageWorkError::Busy)));
        release.release();
        callers.remove(0).await.unwrap().unwrap();
    }
}
