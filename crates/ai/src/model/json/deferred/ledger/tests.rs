use super::*;
#[test]
fn splitting_and_merging_authentic_credit_neither_charge_nor_refund_the_live_heap() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 4096).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut original = ledger.reserve(256).unwrap();
    let mut split = original.split(64).unwrap();
    assert!(split.belongs_to(&ledger));
    assert_eq!(ledger.live_requested_bytes(), baseline + 256);
    original.merge_from(&mut split).unwrap();
    drop(split);
    assert_eq!(ledger.live_requested_bytes(), baseline + 256);
    original.grow(32).unwrap();
    assert_eq!(ledger.live_requested_bytes(), baseline + 288);
    drop(original);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn failed_foreign_merge_and_checked_split_preserve_both_original_reservations() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let original = DecodeLedger::new(&authority, 4096).unwrap();
    let foreign = DecodeLedger::new(&authority, 4096).unwrap();
    let mut left = original.reserve(128).unwrap();
    let mut right = foreign.reserve(64).unwrap();
    let original_live = original.live_requested_bytes();
    let foreign_live = foreign.live_requested_bytes();
    assert!(matches!(
        left.merge_from(&mut right),
        Err(JsonDecodeError::ForeignLedger)
    ));
    assert!(matches!(left.split(129), Err(JsonDecodeError::Arithmetic)));
    assert_eq!(left.bytes(), 128);
    assert_eq!(right.bytes(), 64);
    assert_eq!(original.live_requested_bytes(), original_live);
    assert_eq!(foreign.live_requested_bytes(), foreign_live);
}
