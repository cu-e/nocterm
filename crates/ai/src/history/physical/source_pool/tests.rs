//! Partial conversion and unwinding preserve the original backing lifetime.
use super::*;
use crate::model::{
    json::deferred::DecodeLedger,
    retention::{RetentionAuthority, document::DocumentRetention},
};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn pool(role: &ConstructionRole, ledger: &DecodeLedger, bytes: usize) -> NativeSourcePool {
    let header = NativeCredit::reserve_header(role).unwrap();
    NativeSourcePool::new(role, ledger.reserve(bytes).unwrap(), header)
}

#[test]
fn retiring_one_backing_refunds_it_while_original_metadata_survives() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_foreground().unwrap();
    let ledger = DecodeLedger::for_operation(&role, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let text = "retained".repeat(32);
    let capacity = text.capacity();
    let mut pool = pool(&role, &ledger, capacity);
    let original = pool.original().clone();
    let value = NativeBacking::from_source(text, capacity, &mut pool).unwrap();
    drop(pool);
    assert_eq!(ledger.live_requested_bytes(), baseline + capacity);
    value.retire();
    assert_eq!(ledger.live_requested_bytes(), baseline);
    assert!(value.read().is_none());
    value.retire();
    assert_eq!(ledger.live_requested_bytes(), baseline);
    drop(original);
    drop(value);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

struct CheckedPayload {
    text: String,
    ledger: DecodeLedger,
    baseline: usize,
    panics: bool,
}
impl Drop for CheckedPayload {
    fn drop(&mut self) {
        assert!(self.ledger.live_requested_bytes() >= self.baseline + self.text.capacity());
        assert!(!self.panics, "payload destructor panic");
    }
}

#[test]
fn panicking_payload_destructor_refunds_only_after_unwind_drops_its_fields() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_foreground().unwrap();
    let ledger = DecodeLedger::for_operation(&role, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let text = "panic-owned".repeat(32);
    let capacity = text.capacity();
    let mut pool = pool(&role, &ledger, capacity);
    let payload = CheckedPayload {
        text,
        ledger: ledger.share(),
        baseline,
        panics: true,
    };
    let value = NativeBacking::from_source(payload, capacity, &mut pool).unwrap();
    drop(pool);
    assert!(catch_unwind(AssertUnwindSafe(|| value.retire())).is_err());
    assert_eq!(ledger.live_requested_bytes(), baseline);
    assert!(value.read().is_none());
    drop(value);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn partial_vector_conversion_keeps_leftovers_paid_and_releases_original_vector() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let role = roles.take_foreground().unwrap();
    let ledger = DecodeLedger::for_operation(&role, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut values = Vec::with_capacity(2);
    for _ in 0..2 {
        values.push(CheckedPayload {
            text: "actual backing".repeat(8),
            ledger: ledger.share(),
            baseline,
            panics: false,
        });
    }
    let first_capacity = values[0].text.capacity();
    let bytes = values.capacity() * std::mem::size_of::<CheckedPayload>()
        + values
            .iter()
            .map(|value| value.text.capacity())
            .sum::<usize>();
    let mut pool = pool(&role, &ledger, bytes);
    let mut source = SourceVector::new(values, &mut pool).unwrap();
    let first =
        NativeBacking::from_source(source.next().unwrap(), first_capacity, &mut pool).unwrap();
    // A conversion refusal drops the unconsumed source before its residual pool.
    drop(source);
    drop(pool);
    assert_eq!(ledger.live_requested_bytes(), baseline + first_capacity);
    first.retire();
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
