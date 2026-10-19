use super::*;
#[test]
fn deep_chain_frontier_is_one_and_wide_value_children_set_the_frontier() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 4 * 1024 * 1024).unwrap();
    let mut chain = LiteralFrontier::new(&ledger).unwrap();
    for _ in 0..12000 {
        chain.start().unwrap();
    }
    chain.scalar().unwrap();
    for _ in 0..12000 {
        chain.end().unwrap();
    }
    assert_eq!(chain.finish().unwrap(), 1);
    let mut wide = LiteralFrontier::new(&ledger).unwrap();
    wide.start().unwrap();
    for _ in 0..7 {
        wide.scalar().unwrap();
    }
    wide.end().unwrap();
    assert_eq!(wide.finish().unwrap(), 7);
}
#[test]
fn pending_prior_siblings_are_counted_with_the_completed_child_frontier() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 65536).unwrap();
    let mut frontier = LiteralFrontier::new(&ledger).unwrap();
    frontier.start().unwrap();
    frontier.scalar().unwrap();
    frontier.start().unwrap();
    for _ in 0..4 {
        frontier.scalar().unwrap();
    }
    frontier.end().unwrap();
    frontier.end().unwrap();
    assert_eq!(frontier.finish().unwrap(), 5);
}
