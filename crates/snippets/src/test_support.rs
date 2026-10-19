//! Deterministic draft identities shared by feature and application fixtures.
use crate::SnippetIds;
use std::sync::atomic::{AtomicU64, Ordering};
pub struct SequenceIds {
    first: u128,
    calls: AtomicU64,
}
impl SequenceIds {
    pub fn new(first: u128) -> Self {
        Self {
            first,
            calls: AtomicU64::new(0),
        }
    }
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}
impl SnippetIds for SequenceIds {
    fn next(&self) -> uuid::Uuid {
        uuid::Uuid::from_u128(
            self.first
                .wrapping_add(u128::from(self.calls.fetch_add(1, Ordering::SeqCst))),
        )
    }
}
