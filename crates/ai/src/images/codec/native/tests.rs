//! Native operation hooks and original typed error owners retain exact payments.
use super::*;
use crate::{
    history::source::SourceProducer,
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use std::error::Error;
#[test]
fn native_scope_keeps_first_refusal_and_retires_only_physically_dropped_backings() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (document, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let baseline = document.retained_and_pending_bytes().unwrap();
    let scope = NativeImageAllocationScope::begin(context.ledger()).unwrap();
    scope.admit_before_native_allocation(4096).unwrap();
    let bytes = Vec::<u8>::with_capacity(4096);
    assert_eq!(
        document.retained_and_pending_bytes().unwrap(),
        baseline + bytes.capacity()
    );
    assert!(
        scope
            .admit_before_native_allocation(crate::history::MAX_RESIDENT_BYTES)
            .is_err()
    );
    assert!(scope.admit_before_native_allocation(0).is_err());
    drop(bytes);
    scope.retire_after_native_drop(4096);
    assert!(matches!(
        scope.finish(),
        Err(NativeImageScopeError::Allocation(
            NativeAllocationError::Limit {
                requested: crate::history::MAX_RESIDENT_BYTES,
                ..
            }
        ))
    ));
    assert_eq!(document.retained_and_pending_bytes().unwrap(), baseline);
}
#[test]
fn native_error_ticket_refunds_unused_nested_layout_and_keeps_original_cause_until_last_drop() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let baseline = context.ledger().live_requested_bytes();
    let possible_nested = std::mem::size_of::<std::io::Error>();
    let slot =
        NativeImageErrorSlot::<std::io::Error>::prepare(context.ledger(), possible_nested).unwrap();
    let prepared = context.ledger().live_requested_bytes();
    drop(lease);
    let cause = slot.capture_with_nested(std::io::Error::from_raw_os_error(13), 0);
    assert_eq!(
        context.ledger().live_requested_bytes(),
        prepared - possible_nested
    );
    let pointer = cause
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap() as *const _;
    let clone = cause.clone();
    assert_eq!(
        clone
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _,
        pointer
    );
    assert_eq!(
        clone
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(13)
    );
    drop(cause);
    assert_eq!(
        context.ledger().live_requested_bytes(),
        prepared - possible_nested
    );
    drop(clone);
    assert_eq!(context.ledger().live_requested_bytes(), baseline);
}
