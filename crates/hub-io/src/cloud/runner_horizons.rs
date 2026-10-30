//! One retry horizon per publication, fixed across its attempts.
//!
//! A hub with receipt reclaim drops the receipt of a publication once its `retry_until`
//! horizon has passed. A horizon recomputed on every attempt would keep sliding forward and
//! break the runner's promise not to retry after it, so the first attempt's horizon is kept
//! for the later ones. Only a horizon that has already passed is replaced, because the hub
//! refuses one that is not ahead of its clock. The ledger is bounded and lives with the
//! process: persisting a horizon with the outbox entry needs a journal schema change.
use std::{collections::BTreeMap, sync::Mutex};

use super::runner_reclaim::retry_until;

/// Publications tracked at once; the oldest horizon makes room for a new one.
const MAX_TRACKED: usize = 4096;

#[derive(Default)]
pub(super) struct RetryHorizons {
    entries: Mutex<BTreeMap<[u8; 16], i64>>,
}

impl RetryHorizons {
    /// The horizon for an attempt to publish `delivery` at `now_ms`; `0` (none) unless the
    /// hub negotiated reclaim.
    pub(super) fn horizon(&self, delivery: [u8; 16], now_ms: i64, supported: bool) -> i64 {
        if !supported {
            return 0;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&fixed) = entries.get(&delivery).filter(|&&fixed| fixed > now_ms) {
            return fixed;
        }
        if entries.len() >= MAX_TRACKED && !entries.contains_key(&delivery) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, horizon)| **horizon)
                .map(|(delivery, _)| *delivery);
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        let horizon = retry_until(now_ms, true);
        entries.insert(delivery, horizon);
        horizon
    }

    /// Forgets a publication the hub acknowledged.
    pub(super) fn settle(&self, delivery: &[u8; 16]) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(delivery);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_publication_keeps_its_first_horizon_across_attempts() {
        let horizons = RetryHorizons::default();
        let first = horizons.horizon([1; 16], 1_000, true);
        assert!(first > 1_000);
        assert_eq!(horizons.horizon([1; 16], 5_000, true), first);
        assert_ne!(horizons.horizon([2; 16], 5_000, true), first);
    }

    #[test]
    fn an_old_hub_gets_no_horizon_and_nothing_is_tracked() {
        let horizons = RetryHorizons::default();
        assert_eq!(horizons.horizon([1; 16], 1_000, false), 0);
        assert!(horizons.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn a_passed_horizon_is_replaced_and_an_acknowledged_one_is_forgotten() {
        let horizons = RetryHorizons::default();
        let first = horizons.horizon([1; 16], 1_000, true);
        let later = horizons.horizon([1; 16], first + 1, true);
        assert!(later > first);
        horizons.settle(&[1; 16]);
        assert!(horizons.horizon([1; 16], 2_000, true) < later);
    }

    #[test]
    fn the_ledger_stays_bounded_by_dropping_the_oldest_horizon() {
        let horizons = RetryHorizons::default();
        for n in 0..=MAX_TRACKED as u64 {
            let mut delivery = [0; 16];
            delivery[..8].copy_from_slice(&n.to_be_bytes());
            horizons.horizon(delivery, i64::try_from(n).unwrap(), true);
        }
        assert_eq!(horizons.entries.lock().unwrap().len(), MAX_TRACKED);
        assert!(!horizons.entries.lock().unwrap().contains_key(&[0; 16]));
    }
}
