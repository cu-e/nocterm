//! Same original engine transitions with actual prepaid cache/search ownership.
use super::*;
use alloc::vec;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
#[derive(Debug)]
struct Refused;
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("original test refusal")
    }
}
impl std::error::Error for Refused {}
#[derive(Clone)]
struct Meter {
    live: Arc<AtomicUsize>,
    limit: Arc<AtomicUsize>,
}
struct Credit {
    bytes: usize,
    meter: Meter,
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.meter.live.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}
impl Budget for Meter {
    type Credit = Credit;
    type Error = Refused;
    fn reserve(&self, layout: Layout) -> Result<Credit, Refused> {
        let live = self.live.load(Ordering::SeqCst);
        if live
            .checked_add(layout.size())
            .map_or(true, |value| value > self.limit.load(Ordering::SeqCst))
        {
            return Err(Refused);
        }
        self.live.fetch_add(layout.size(), Ordering::SeqCst);
        Ok(Credit {
            bytes: layout.size(),
            meter: self.clone(),
        })
    }
}
fn meter() -> Meter {
    Meter {
        live: Arc::new(AtomicUsize::new(0)),
        limit: Arc::new(AtomicUsize::new(usize::MAX)),
    }
}
#[test]
fn managed_scratch_preserves_original_unicode_empty_capture_and_leftmost_searches() {
    for pattern in [
        r"(?i)\b(authorization|proxy-authorization)\s*:\s*(?:bearer|basic|token)?\s*[^\s,;]+",
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)",
        r"(a|ab)+",
        r"(?u)\b",
        r"(?:|Ω)",
    ] {
        let re = PikeVM::new(pattern).unwrap();
        let meter = meter();
        let mut native = ManagedCache::new(&re, meter.clone()).unwrap();
        let mut original = re.create_cache();
        for text in [
            "",
            "Ω",
            "aaab",
            "Authorization: Bearer secret",
            "-----BEGIN RSA PRIVATE KEY-----\nΩ secret",
            "é Ω z",
        ] {
            let input = Input::new(text);
            let mut slots = vec![None; native.slots().len()];
            let expected = re.search_slots(&mut original, &input, &mut slots);
            assert_eq!(
                native.search(&input).unwrap(),
                expected,
                "{pattern}: {text}"
            );
            if expected.is_some() {
                assert_eq!(native.slots(), slots, "{pattern}: {text}");
            }
            assert_eq!(native.memory_usage(), meter.live.load(Ordering::SeqCst));
        }
        drop(native);
        assert_eq!(meter.live.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn original_reservation_refusal_cleans_partial_search_and_same_cache_retries() {
    let re = PikeVM::new(r"((a|ab|ac|ad)+)").unwrap();
    let meter = meter();
    let mut cache = ManagedCache::new(&re, meter.clone()).unwrap();
    let fixed = meter.live.load(Ordering::SeqCst);
    meter.limit.store(fixed, Ordering::SeqCst);
    assert!(matches!(
        cache.search(&Input::new("aacad")),
        Err(Error::Refused(Refused))
    ));
    assert_eq!(meter.live.load(Ordering::SeqCst), fixed);
    meter.limit.store(usize::MAX, Ordering::SeqCst);
    assert!(cache.search(&Input::new("aacad")).unwrap().is_some());
    drop(cache);
    assert_eq!(meter.live.load(Ordering::SeqCst), 0);
}
#[test]
fn optional_growth_headroom_refusal_falls_back_to_actual_needed_capacity() {
    let meter = meter();
    let mut stack = Vec::new();
    let mut receipt = None;
    {
        let mut growth = Controlled {
            budget: &meter,
            credit: &mut receipt,
            failure: None,
        };
        assert!(growth.reserve(&mut stack, 3));
    }
    for _ in 0..3 {
        stack.push(FollowEpsilon::Explore(StateID::ZERO));
    }
    meter
        .limit
        .store(7 * core::mem::size_of::<FollowEpsilon>(), Ordering::SeqCst);
    {
        let mut growth = Controlled {
            budget: &meter,
            credit: &mut receipt,
            failure: None,
        };
        assert!(growth.reserve(&mut stack, 1));
        assert!(growth.failure.is_none());
    }
    assert_eq!(stack.capacity(), 4);
    assert_eq!(stack.len(), 3);
    drop(stack);
    drop(receipt);
    assert_eq!(meter.live.load(Ordering::SeqCst), 0);
}
