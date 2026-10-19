//! Actual shared JSON aliases keep original admission credit until real retirement.
use crate::model::json::{
    OwnedJson,
    deferred::{
        DecodeLedger, JsonDecodeError,
        literal::{ExactLiteralText, LiteralBuilder},
    },
};
use std::sync::{Arc, Barrier};

#[test]
fn controlled_alias_refuses_same_ledger_reuse_until_payload_retires() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    const LIMIT: usize = 16384;
    let ledger = DecodeLedger::new(&authority, LIMIT).unwrap();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    let mut text = builder.exact_text(512).unwrap();
    text.append(&"x".repeat(512)).unwrap();
    builder.string(text.finish().unwrap()).unwrap();
    let decoded = builder.finish().unwrap();
    let alias = decoded.as_owned().clone();
    let receipt = alias.receipt().clone();
    let sibling = LiteralBuilder::new(&ledger).unwrap();
    let requested =
        LIMIT - ledger.live_requested_bytes() - std::mem::size_of::<ExactLiteralText>() + 1;
    drop(decoded);
    assert!(receipt.matches(&alias));
    assert_eq!(alias.as_value().as_str().unwrap().len(), 512);
    assert!(matches!(
        sibling.exact_text(requested),
        Err(JsonDecodeError::Limit { .. })
    ));
    drop(alias);
    let admitted = sibling.exact_text(requested).unwrap();
    drop(admitted);
    drop(sibling);
    drop(receipt);
}

#[test]
fn concurrent_last_real_alias_and_weak_receipt_drop_return_credit_once() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnedJson>();
    send_sync::<DecodeLedger>();
    let ledger = DecodeLedger::new(&authority, 16384).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&ledger).unwrap();
    let mut text = builder.text().unwrap();
    text.append("shared authentic payload").unwrap();
    builder.string(text).unwrap();
    let decoded = builder.finish().unwrap();
    let left = decoded.as_owned().clone();
    let right = left.clone();
    let receipt = left.receipt().clone();
    let identity = receipt.clone();
    drop(decoded);
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let gate = barrier.clone();
        scope.spawn(move || {
            gate.wait();
            drop(left);
            drop(receipt);
        });
        let gate = barrier.clone();
        scope.spawn(move || {
            gate.wait();
            drop(right);
        });
        barrier.wait();
    });
    assert_eq!(
        ledger.live_requested_bytes(),
        baseline + identity.retained_identity_bytes().unwrap()
    );
    drop(identity);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn foreign_ledger_refusal_keeps_original_backing_live_until_its_alias_drops() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let original = DecodeLedger::new(&authority, 4096).unwrap();
    let foreign = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = original.live_requested_bytes();
    let foreign_baseline = foreign.live_requested_bytes();
    let mut builder = LiteralBuilder::new(&original).unwrap();
    let mut text = builder.text().unwrap();
    text.append("original allocation").unwrap();
    builder.string(text).unwrap();
    let decoded = builder.finish().unwrap();
    let alias = decoded.as_owned().clone();
    let live = original.live_requested_bytes();
    assert!(matches!(
        decoded.into_owned(&foreign),
        Err(JsonDecodeError::ForeignLedger)
    ));
    assert_eq!(original.live_requested_bytes(), live);
    assert_eq!(foreign.live_requested_bytes(), foreign_baseline);
    assert_eq!(alias.as_value().as_str(), Some("original allocation"));
    drop(alias);
    assert_eq!(original.live_requested_bytes(), baseline);
}
