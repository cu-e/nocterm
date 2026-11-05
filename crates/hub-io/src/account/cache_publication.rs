//! An unpublished cache candidate retains its exact original content generation.
//! Staging has no authority; only Slot can atomically admit its namespace commit.
use std::sync::Arc;

use nocterm_core::fs::{AtomicPublication, CommittedPublication};

use crate::cache::{AccountCache, CacheError};

pub(crate) struct CacheDraft {
    original: Option<Arc<AccountCache>>,
    next: Arc<AccountCache>,
    publication: AtomicPublication,
}

impl CacheDraft {
    pub(super) fn new(
        original: Option<Arc<AccountCache>>,
        next: AccountCache,
        publication: AtomicPublication,
    ) -> Self {
        Self {
            original,
            next: Arc::new(next),
            publication,
        }
    }

    pub(super) fn account(&self) -> &str {
        &self.next.account_id
    }

    pub(super) fn matches(&self, current: &Option<Arc<AccountCache>>) -> bool {
        same_generation(&self.original, current)
    }

    pub(super) fn commit(self) -> Result<(Arc<AccountCache>, CommittedPublication), CacheError> {
        Ok((self.next, self.publication.commit()?))
    }
}

pub(super) fn same_generation(
    original: &Option<Arc<AccountCache>>,
    current: &Option<Arc<AccountCache>>,
) -> bool {
    match (original, current) {
        (Some(original), Some(current)) => Arc::ptr_eq(original, current),
        (None, None) => true,
        _ => false,
    }
}
