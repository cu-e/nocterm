use nocterm_session::local_fs::{CancelFlag, LocalResult};
use nocterm_transfers::TransferCause;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// The permit travels into the blocking closure. Cancelling its waiter cannot
/// admit additional work while a kernel operation still occupies the slot.
pub(crate) struct BlockingDispatcher {
    slots: Arc<Semaphore>,
}
impl BlockingDispatcher {
    pub(crate) fn new(slots: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
        }
    }
    pub(crate) async fn call<T, F>(
        &self,
        cancel: CancelFlag,
        operation: F,
    ) -> Result<T, TransferCause>
    where
        T: Send + 'static,
        F: FnOnce() -> LocalResult<T> + Send + 'static,
    {
        cancel.check()?;
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(TransferCause::runtime)?;
        cancel.check()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            cancel.check()?;
            operation()
        })
        .await
        .map_err(TransferCause::runtime)?
        .map_err(Into::into)
    }
    /// Cleanup also runs off executor threads, even after cancellation.
    pub(crate) async fn cleanup<T, F>(&self, operation: F) -> Result<T, TransferCause>
    where
        T: Send + 'static,
        F: FnOnce() -> LocalResult<T> + Send + 'static,
    {
        self.call(CancelFlag::default(), operation).await
    }
}
