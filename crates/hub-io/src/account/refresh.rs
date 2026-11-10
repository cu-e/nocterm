//! Only a successful concurrent personal refresh satisfies waiting readers.
use super::OpenKeys;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

#[derive(Default)]
pub(super) struct Refresh {
    serial: tokio::sync::Mutex<()>,
    completed: AtomicU64,
    completed_keys: Mutex<Weak<OpenKeys>>,
}

pub(crate) struct RefreshTurn<'a> {
    owner: &'a Refresh,
    _guard: tokio::sync::MutexGuard<'a, ()>,
    needed: bool,
    keys: Weak<OpenKeys>,
}

impl Refresh {
    pub(super) async fn enter(&self, keys: &Arc<OpenKeys>) -> RefreshTurn<'_> {
        let before = self.completed.load(Ordering::Acquire);
        let guard = self.serial.lock().await;
        let same_keys = self
            .completed_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .upgrade()
            .is_some_and(|completed| Arc::ptr_eq(&completed, keys));
        RefreshTurn {
            owner: self,
            _guard: guard,
            needed: !same_keys || before == self.completed.load(Ordering::Acquire),
            keys: Arc::downgrade(keys),
        }
    }
}

impl RefreshTurn<'_> {
    pub(crate) fn needed(&self) -> bool {
        self.needed
    }

    pub(crate) fn complete(self) {
        if self.needed {
            *self
                .owner
                .completed_keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = self.keys;
            self.owner.completed.fetch_add(1, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::testing::open_keys;
    use futures::{FutureExt as _, poll};

    #[tokio::test]
    async fn waiting_readers_reuse_only_a_successful_refresh_of_the_same_open_keys() {
        let refresh = Refresh::default();
        let keys = Arc::new(open_keys());
        let leader = refresh.enter(&keys).await;
        assert!(leader.needed());
        let mut same = refresh.enter(&keys).boxed();
        let replacement = Arc::new(open_keys());
        let mut next = refresh.enter(&replacement).boxed();
        assert!(poll!(&mut same).is_pending());
        assert!(poll!(&mut next).is_pending());
        leader.complete();
        let follower = same.await;
        assert!(!follower.needed());
        follower.complete();
        assert!(next.await.needed());
    }

    #[tokio::test]
    async fn cancelled_or_failed_leaders_do_not_satisfy_waiting_readers() {
        let refresh = Refresh::default();
        let keys = Arc::new(open_keys());
        let leader = refresh.enter(&keys).await;
        let mut waiting = refresh.enter(&keys).boxed();
        assert!(poll!(&mut waiting).is_pending());
        drop(leader);
        let retry = waiting.await;
        assert!(retry.needed());
        retry.complete();
        assert!(refresh.enter(&keys).await.needed());
    }
}
