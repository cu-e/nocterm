//! Physical ownership, refusal atomicity and original-source behavior.
use super::*;
use std::{
    alloc::Layout,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Meter(Arc<Mutex<Usage>>);
#[derive(Default)]
struct Usage {
    live: usize,
    peak: usize,
    limit: usize,
    reservations: usize,
}
struct Credit {
    meter: Meter,
    bytes: usize,
}
#[derive(Debug)]
struct Refused;
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("test refusal")
    }
}
impl Error for Refused {}
impl Budget for Meter {
    type Credit = Credit;
    type Error = Refused;
    fn same_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    fn reserve(&self, layout: Layout, _: AllocationKind) -> Result<Credit, Refused> {
        let mut usage = self.0.lock().unwrap();
        if layout.size() > usage.limit.saturating_sub(usage.live) {
            return Err(Refused);
        }
        usage.live += layout.size();
        usage.peak = usage.peak.max(usage.live);
        usage.reservations += 1;
        Ok(Credit {
            meter: self.clone(),
            bytes: layout.size(),
        })
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.meter.0.lock().unwrap().live -= self.bytes;
    }
}
fn meter() -> Meter {
    Meter(Arc::new(Mutex::new(Usage {
        limit: usize::MAX,
        ..Usage::default()
    })))
}

#[test]
fn original_source_spans_alias_backing_and_refund_after_last_owner() {
    let budget = meter();
    let source = Source::copy("aébc", budget.clone()).unwrap();
    let span = source.span(1..3).unwrap();
    let same = span.clone();
    assert_eq!(span.text(), "é");
    assert_eq!(span.text().as_ptr(), same.text().as_ptr());
    assert!(source.same_source(span.source()));
    assert!(source.span(1..2).is_err());
    let paid = budget.0.lock().unwrap().live;
    assert!(paid >= 5);
    drop(source);
    drop(span);
    assert_eq!(budget.0.lock().unwrap().live, paid);
    drop(same);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn weak_header_stays_paid_after_payload_destruction() {
    let budget = meter();
    let strong = Shared::new(17u64, &budget).unwrap();
    let weak = strong.downgrade();
    let paid = budget.0.lock().unwrap().live;
    assert!(weak.upgrade().is_some());
    drop(strong);
    assert!(weak.upgrade().is_none());
    assert_eq!(budget.0.lock().unwrap().live, paid);
    drop(weak);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn refusal_does_not_consume_values_or_refund_old_backing() {
    let budget = meter();
    let mut values = Buffer::new(budget.clone());
    values.push(7u64).unwrap();
    let paid = budget.0.lock().unwrap().live;
    budget.0.lock().unwrap().limit = paid;
    let (error, rejected) = values.push(9).unwrap_err();
    assert!(matches!(error, ManagedError::Refused(_)));
    assert_eq!(rejected, 9);
    assert_eq!(values.as_slice(), &[7]);
    assert_eq!(budget.0.lock().unwrap().live, paid);
    drop(values);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn replacement_prepays_simultaneously_live_old_and_new_backing() {
    let budget = meter();
    let mut values = Buffer::new(budget.clone());
    values.push(7u64).unwrap();
    values.push(9).unwrap();
    assert_eq!(values.as_slice(), &[7, 9]);
    assert_eq!(budget.0.lock().unwrap().peak, 24);
    assert_eq!(budget.0.lock().unwrap().live, 16);
    drop(values);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn invalid_utf8_releases_original_backing_and_credit() {
    let budget = meter();
    let mut bytes = Source::buffer(budget.clone());
    bytes.extend_bytes(&[0xff]).unwrap();
    assert!(matches!(
        Source::from_buffer(bytes),
        Err(ManagedError::InvalidUtf8)
    ));
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn registry_rehash_preserves_values_and_removal_chains() {
    let budget = meter();
    let mut map = super::map::Map::new(budget.clone());
    for key in 0..1024 {
        map.reserve().unwrap();
        assert_eq!(map.insert_reserved(key, key * 3), None);
    }
    for key in (0..1024).step_by(2) {
        assert_eq!(map.remove(&key), Some(key * 3));
    }
    for key in (1..1024).step_by(2) {
        assert_eq!(map.remove(&key), Some(key * 3));
    }
    assert_eq!(map.values().count(), 0);
    assert_eq!(map.drain().count(), 0);
    drop(map);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn partial_registry_drain_leaves_reusable_empty_registry() {
    let budget = meter();
    let mut map = super::map::Map::new(budget.clone());
    for key in 0..7 {
        map.reserve().unwrap();
        map.insert_reserved(key, key);
    }
    let mut drained = map.drain();
    assert!(drained.next().is_some());
    drop(drained);
    map.reserve().unwrap();
    map.insert_reserved(20, 30);
    assert_eq!(map.remove(&20), Some(30));
    assert_eq!(map.values().count(), 0);
    drop(map);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn incremental_growth_is_amortized_and_headroom_refusal_can_use_exact_backing() {
    let budget = meter();
    let mut buffer = Buffer::new(budget.clone());
    for value in 0u64..4096 {
        buffer.push(value).unwrap();
    }
    assert!(budget.0.lock().unwrap().reservations <= 13);
    drop(buffer);
    let mut buffer = Buffer::new(budget.clone());
    buffer.reserve(2).unwrap();
    buffer.push(1u64).unwrap();
    buffer.push(2).unwrap();
    // Existing16 + exact replacement24 fits; doubled replacement32 does not.
    budget.0.lock().unwrap().limit = 40;
    buffer.push(3).unwrap();
    assert_eq!(buffer.as_slice(), &[1, 2, 3]);
    assert_eq!(buffer.capacity(), 3);
    drop(buffer);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

mod routing;

#[test]
fn paid_index_replaces_without_growth_and_returns_refused_inputs() {
    let budget = meter();
    let mut index = Index::new(budget.clone());
    index.insert(1u64, 10u64).unwrap();
    let paid = budget.0.lock().unwrap().live;
    budget.0.lock().unwrap().limit = paid;
    assert_eq!(index.insert(1, 20).unwrap(), Some(10));
    assert_eq!(index.get(&1), Some(&20));
    let (error, key, value) = index.insert(2, 30).unwrap_err();
    assert!(matches!(error, ManagedError::Refused(_)));
    assert_eq!((key, value), (2, 30));
    assert_eq!(index.get(&1), Some(&20));
    assert_eq!(index.get(&2), None);
    *index.get_mut(&1).unwrap() = 40;
    drop(index);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn paid_index_borrowed_queries_preserve_hash_contract_without_probe_allocations() {
    use std::{
        borrow::Borrow,
        hash::{Hash, Hasher},
    };
    struct Key(Source<Meter>);
    impl Borrow<str> for Key {
        fn borrow(&self) -> &str {
            self.0.text()
        }
    }
    impl Hash for Key {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.text().hash(state);
        }
    }
    impl PartialEq for Key {
        fn eq(&self, other: &Self) -> bool {
            self.0.text() == other.0.text()
        }
    }
    impl Eq for Key {}
    let budget = meter();
    let mut index = Index::new(budget.clone());
    for (id, value) in [(" 001 ", 7), ("é😀", 9), ("001", 11)] {
        assert!(
            index
                .insert(Key(Source::copy(id, budget.clone()).unwrap()), value)
                .is_ok()
        );
    }
    let before = budget.0.lock().unwrap().reservations;
    assert_eq!(index.get(" 001 "), Some(&7));
    assert_eq!(index.get("001"), Some(&11));
    *index.get_mut("é😀").unwrap() = 13;
    assert_eq!(index.remove("é😀"), Some(13));
    assert_eq!(index.get("é😀"), None);
    assert_eq!(budget.0.lock().unwrap().reservations, before);
    drop(index);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
