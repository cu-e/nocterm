//! One monotonic invalidation signal; notifications coalesce and carry no transfer data.
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Notify;
#[derive(Default)]
pub(crate) struct Changes {
    revision: AtomicU64,
    changed: Notify,
}
impl Changes {
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub(crate) fn advance(&self) {
        let _ = self
            .revision
            .fetch_update(Ordering::Release, Ordering::Relaxed, |revision| {
                Some(revision.saturating_add(1))
            });
        self.changed.notify_waiters();
    }
    pub(crate) async fn since(&self, revision: u64) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.revision() != revision || revision == u64::MAX {
                return;
            }
            notified.await;
        }
    }
}
