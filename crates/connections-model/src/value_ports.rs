//! Supplied identifiers and timestamps; no entropy, environment or clock discovery.
use crate::profiles::ProfileId;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicU64, Ordering},
};
pub trait ProfileIds: Send + Sync + 'static {
    fn next(&self) -> ProfileId;
}
pub trait ConnectionClock: Send + Sync + 'static {
    fn unix_seconds(&self) -> u64;
}
/// Pure deterministic capabilities for memory-only composition.
pub struct SuppliedValues {
    sequence: AtomicU64,
    timestamp: u64,
    reserved: HashSet<ProfileId>,
}
impl SuppliedValues {
    pub fn new(first: u64, timestamp: u64) -> Self {
        Self::excluding(first, timestamp, std::iter::empty())
    }
    /// Every yielded identity skips all supplied preloaded profiles, including later sequence values.
    pub fn excluding(
        first: u64,
        timestamp: u64,
        reserved: impl IntoIterator<Item = ProfileId>,
    ) -> Self {
        Self {
            sequence: AtomicU64::new(first),
            timestamp,
            reserved: reserved.into_iter().collect(),
        }
    }
}
impl ProfileIds for SuppliedValues {
    fn next(&self) -> ProfileId {
        loop {
            let mut bytes = [0; 16];
            bytes[8..]
                .copy_from_slice(&self.sequence.fetch_add(1, Ordering::Relaxed).to_be_bytes());
            let id = ProfileId::from_bytes(bytes);
            if !self.reserved.contains(&id) {
                return id;
            }
        }
    }
}
impl ConnectionClock for SuppliedValues {
    fn unix_seconds(&self) -> u64 {
        self.timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(value: u64) -> ProfileId {
        let mut bytes = [0; 16];
        bytes[8..].copy_from_slice(&value.to_be_bytes());
        ProfileId::from_bytes(bytes)
    }
    #[test]
    fn deterministic_identifiers_skip_all_preloaded_values_on_each_request() {
        let values = SuppliedValues::excluding(1, 42, [id(1), id(3)]);
        assert_eq!(values.next(), id(2));
        assert_eq!(values.next(), id(4));
        assert_eq!(values.unix_seconds(), 42);
    }
}
