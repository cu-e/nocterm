//! Public controlled-builder ownership and cleanup behavior, without private field mutation.
use super::super::{
    OwnedJson,
    deferred::{
        DecodeLedger, JsonDecodeError,
        literal::{ExactLiteralText, LiteralBuilder},
    },
};
fn chain(builder: &mut LiteralBuilder, depth: usize) {
    for _ in 0..depth {
        builder.start_array().unwrap();
    }
    builder.null().unwrap();
    for _ in 0..depth {
        builder.end_container().unwrap();
    }
}
fn key(builder: &mut LiteralBuilder, name: &str) {
    let mut key = builder.text().unwrap();
    key.append(name).unwrap();
    builder.key(key).unwrap();
}
#[test]
fn final_deep_root_and_last_clone_drop_iteratively_with_a_live_weak_identity() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnedJson>();
    let ledger = DecodeLedger::new(&authority, 32 * 1024 * 1024).unwrap();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 1).unwrap();
    chain(&mut builder, 12000);
    let value = builder.finish().unwrap();
    let alias = value.as_owned().clone();
    let receipt = alias.receipt().clone();
    let mut borrowed = alias.as_value();
    for _ in 0..12000 {
        borrowed = &borrowed.as_array().unwrap()[0];
    }
    assert!(borrowed.is_null());
    drop(value);
    assert!(receipt.matches(&alias));
    drop(alias);
    assert_eq!(
        receipt.retained_identity_bytes(),
        OwnedJson::identity_allocation_bytes()
    );
}
#[test]
fn duplicate_replacement_retires_a_deep_old_value_without_changing_key_position() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 32 * 1024 * 1024).unwrap();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 3).unwrap();
    builder.start_object().unwrap();
    key(&mut builder, "first");
    chain(&mut builder, 6000);
    let before = ledger.live_requested_bytes();
    key(&mut builder, "first");
    builder.boolean(true).unwrap();
    assert!(ledger.live_requested_bytes() < before);
    key(&mut builder, "second");
    builder.null().unwrap();
    builder.end_container().unwrap();
    let value = builder.finish().unwrap();
    let object = value.as_owned().as_value().as_object().unwrap();
    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(object["first"], true);
}
#[test]
fn insufficient_admitted_frontier_refuses_before_a_raw_aggregate_and_cleans_children() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16 * 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 1).unwrap();
    builder.start_array().unwrap();
    chain(&mut builder, 3000);
    builder.null().unwrap();
    assert!(matches!(
        builder.end_container(),
        Err(JsonDecodeError::CleanupFrontier)
    ));
    drop(builder);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn canceling_partial_frames_drops_completed_deep_children_under_their_live_reservations() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16 * 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 1).unwrap();
    builder.start_array().unwrap();
    chain(&mut builder, 3000);
    drop(builder);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn actual_backing_refusal_cleans_deep_children_without_a_cleanup_allocation() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    const LIMIT: usize = 1024 * 1024;
    let ledger = DecodeLedger::new(&authority, LIMIT).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::with_cleanup_frontier(&ledger, 1).unwrap();
    builder.start_array().unwrap();
    chain(&mut builder, 128);
    let pressure_builder = LiteralBuilder::new(&ledger).unwrap();
    let available = LIMIT - ledger.live_requested_bytes() - std::mem::size_of::<ExactLiteralText>();
    let pressure = pressure_builder.exact_text(available).unwrap();
    assert_eq!(ledger.live_requested_bytes(), LIMIT);
    assert!(matches!(
        builder.end_container(),
        Err(JsonDecodeError::Limit { .. })
    ));
    drop(pressure);
    drop(pressure_builder);
    drop(builder);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn a_real_cleanup_workspace_keeps_credit_until_its_last_independent_handle_retires() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    use super::Cleanup;
    let ledger = DecodeLedger::new(&authority, 16384).unwrap();
    let baseline = ledger.live_requested_bytes();
    let workspace = Cleanup::admit(13, &ledger).unwrap();
    let clone = workspace.clone();
    let live = ledger.live_requested_bytes();
    assert!(live > baseline);
    drop(workspace);
    assert_eq!(ledger.live_requested_bytes(), live);
    clone.check(13).unwrap();
    drop(clone);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
