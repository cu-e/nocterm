//! Cancellation is an uncertain credential mutation, never proof of completion.
use crate::RunnerError;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
const IDLE: u8 = 0;
const PENDING: u8 = 1;
const UNKNOWN: u8 = 2;
#[derive(Default)]
pub(super) struct CredentialOperations {
    state: AtomicU8,
}
impl CredentialOperations {
    pub(super) fn completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == IDLE
    }
    pub(super) async fn observe<T>(
        self: &Arc<Self>,
        operation: impl Future<Output = Result<T, RunnerError>>,
    ) -> Result<T, RunnerError> {
        self.state
            .compare_exchange(IDLE, PENDING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| RunnerError::DrainTimeout)?;
        let mut guard = PendingOperation {
            owner: self.clone(),
            observed: false,
        };
        let result = operation.await;
        if result.is_ok() {
            guard.observed = true;
            self.state.store(IDLE, Ordering::Release);
        }
        result
    }
}
struct PendingOperation {
    owner: Arc<CredentialOperations>,
    observed: bool,
}
impl Drop for PendingOperation {
    fn drop(&mut self) {
        if !self.observed {
            self.owner.state.store(UNKNOWN, Ordering::Release);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;
    #[tokio::test]
    async fn runner_credential_completion_does_not_treat_cancelled_or_failed_mutations_as_drained()
    {
        let owner = Arc::new(CredentialOperations::default());
        let (entered, observed) = oneshot::channel();
        let (release, held) = oneshot::channel();
        let task_owner = owner.clone();
        let task = tokio::spawn(async move {
            task_owner
                .observe(async {
                    let _ = entered.send(());
                    held.await.map_err(|_| RunnerError::Timeout)?;
                    Ok(())
                })
                .await
        });
        observed.await.unwrap();
        assert!(!owner.completed());
        release.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(owner.completed());

        let (entered, observed) = oneshot::channel();
        let (_release, held) = oneshot::channel::<()>();
        let task_owner = owner.clone();
        let task = tokio::spawn(async move {
            task_owner
                .observe(async {
                    let _ = entered.send(());
                    held.await.map_err(|_| RunnerError::Timeout)?;
                    Ok(())
                })
                .await
        });
        observed.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!owner.completed());
        assert!(owner.observe(async { Ok(()) }).await.is_err());
        assert!(!owner.completed());

        let failed = Arc::new(CredentialOperations::default());
        assert!(
            failed
                .observe(async { Err::<(), _>(RunnerError::Timeout) })
                .await
                .is_err()
        );
        assert!(!failed.completed());
    }
}
