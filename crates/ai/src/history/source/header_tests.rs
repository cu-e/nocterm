//! Source metadata stays charged until its original physical endpoint retires.
use super::*;
use crate::{
    history::physical::SnapshotBuilder,
    model::{
        allocations::shared_inline_bytes,
        native::NativeBytes,
        retention::{
            RetentionAuthority,
            document::{
                DocumentRetention,
                tests::{snapshot, text},
            },
            graph,
        },
    },
};

// Borrowed genuine ledgers let the ordinary sealed census deduplicate the
// authority/account shared with the new source. This wrapper owns no payload.
struct LedgerPair<'a>(&'a DecodeLedger, &'a DecodeLedger);
impl graph::sealed::Source for LedgerPair<'_> {}
impl graph::OwnerGraph for LedgerPair<'_> {
    fn count(&self, count: &mut graph::Count<'_, '_>) -> Result<(), RetentionError> {
        graph::OwnerGraph::count(self.0, count)?;
        graph::OwnerGraph::count(self.1, count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        graph::OwnerGraph::cleanup(self.0, permit);
        graph::OwnerGraph::cleanup(self.1, permit);
    }
    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        graph::OwnerGraph::adopt(self.0, publication);
        graph::OwnerGraph::adopt(self.1, publication);
    }
}

#[test]
fn source_context_and_ledger_headers_retire_after_adopted_rows_are_removed() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    foreground.clear_source();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let ledger_header = context.ledger().live_requested_bytes();

    let row = crate::history::native::entries::agent(text(context.ledger(), "original source row"));
    let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
    builder.append_row(row, 2).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    foreground.clear_source();
    assert_eq!(live.snapshot().unwrap().rows().len(), 2);

    let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
    assert!(builder.truncate_rows(0).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    foreground.clear_source();
    assert!(live.snapshot().unwrap().rows().is_empty());
    assert_eq!(context.ledger().live_requested_bytes(), ledger_header);

    drop(lease);
    let before = scope.retained_and_pending_bytes().unwrap();
    drop(context);
    let after = scope.retained_and_pending_bytes().unwrap();
    assert_eq!(
        before - after,
        shared_inline_bytes::<ContextBody>() + ledger_header,
        "the producer retains the current source phase, but not old context or ledger headers",
    );
    assert!(producer.capture().is_ok());
}

#[test]
fn retired_source_phase_and_ledger_remain_charged_until_original_payload_drops() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let old_lease = producer.capture().unwrap();
    let old_context = old_lease.context().unwrap();
    let mut bytes = NativeBytes::new(old_context.ledger(), 4096).unwrap();
    bytes.append(b"original payload").unwrap();
    let payload = bytes.finish();
    drop(old_lease);

    let new_lease = producer.capture().unwrap();
    let new_context = new_lease.context().unwrap();
    assert!(old_context.same_document(&new_context));
    assert!(!old_context.same_source(&new_context));
    let new_baseline = new_context.ledger().live_requested_bytes();
    let preserved = NativeBytes::new(new_context.ledger(), 2048).unwrap();
    let current_metadata = graph::measure(&authority, new_context.ledger()).unwrap();
    let both_metadata = graph::measure(
        &authority,
        &LedgerPair(new_context.ledger(), old_context.ledger()),
    )
    .unwrap();
    let old_ledger_and_phase = both_metadata - current_metadata;
    assert!(old_ledger_and_phase > old_context.ledger().live_requested_bytes() - 4096);
    let before_context = scope.retained_and_pending_bytes().unwrap();
    drop(old_context);
    let after_context = scope.retained_and_pending_bytes().unwrap();
    assert_eq!(
        before_context - after_context,
        shared_inline_bytes::<ContextBody>(),
        "the surviving payload still owns its original ledger and retired phase",
    );
    assert_eq!(
        new_context.ledger().live_requested_bytes(),
        new_baseline + 2048
    );

    drop(payload);
    let after_payload = scope.retained_and_pending_bytes().unwrap();
    assert_eq!(
        after_context - after_payload,
        4096 + old_ledger_and_phase,
        "the last original payload frees its bytes and actual ledger/phase metadata",
    );
    assert_eq!(
        new_context.ledger().live_requested_bytes(),
        new_baseline + 2048
    );
    drop(preserved);
    assert_eq!(new_context.ledger().live_requested_bytes(), new_baseline);
    assert!(new_context.reserve_native(Layout::new::<u64>()).is_ok());
}
