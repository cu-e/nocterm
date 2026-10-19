use nocterm_transfers::*;
#[test]
fn short_growth_and_oversized_chunks_refuse_publication() {
    let mut budget = StreamBudget::new(StreamLimits::default(), Some(8));
    budget.accept_chunk(4).unwrap();
    budget.acknowledged(4).unwrap();
    assert!(matches!(
        budget.finish(),
        Err(TransferCause::SizeChanged { .. })
    ));
    assert!(matches!(
        budget.accept_chunk(5),
        Err(TransferCause::SizeChanged { .. })
    ));
    assert!(matches!(
        budget.accept_chunk(32769),
        Err(TransferCause::OversizedChunk { .. })
    ));
    budget.acknowledged(4).unwrap();
    budget.finish().unwrap();
}
#[test]
fn unacknowledged_batch_bytes_also_obey_the_known_size() {
    let budget = StreamBudget::new(StreamLimits::default(), Some(8));
    budget.accept_buffered(4, 4).unwrap();
    assert!(matches!(
        budget.accept_buffered(4, 5),
        Err(TransferCause::SizeChanged { observed: 9, .. })
    ));
    assert_eq!(budget.transferred(), 0);
}
#[test]
fn unknown_size_streams_count_acknowledged_bytes_and_finish() {
    let mut budget = StreamBudget::new(StreamLimits::default(), None);
    budget.accept_chunk(32768).unwrap();
    assert_eq!(budget.transferred(), 0);
    budget.acknowledged(32768).unwrap();
    budget.finish().unwrap();
    assert_eq!(budget.transferred(), 32768);
}
