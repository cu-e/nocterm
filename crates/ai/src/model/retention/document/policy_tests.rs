//! Original producer actions preserve maximum-root policy and closed payload kinds.
use super::tests::{snapshot, text};
use super::*;
use crate::{
    history::physical::{RowBody, SnapshotBuilder},
    model::json::deferred::DecodeLedger,
};

#[test]
fn alternate_large_snapshots_share_one_policy_and_weak_headers_retire_it() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = roles.take_foreground().unwrap();
    let mut reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    let mut latest = roles.take_writer_latest().unwrap();
    let mut candidate = roles.take_candidate().unwrap();
    let mut delivery = roles.take_delivery().unwrap();
    let large = "x".repeat(20 * 1024 * 1024);
    live.replace(snapshot(&foreground, &large)).unwrap();
    drop(large);
    let original = live.snapshot().unwrap().policy_bytes().unwrap();
    assert!(original > crate::history::MAX_RESIDENT_BYTES / 2);
    let one = scope.retained_and_pending_bytes().unwrap();
    for role in [&mut active, &mut latest, &mut candidate, &mut delivery] {
        role.clone_from_role(&live).unwrap();
        assert_eq!(scope.retained_and_pending_bytes().unwrap(), one);
    }
    let pointer = match live.snapshot().unwrap().rows()[0].view() {
        RowBody::Agent(value) => value.read().as_ptr(),
        _ => panic!("agent row"),
    };
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let title = crate::model::native::NativeOwned::supplied(text(&ledger, "new title"));
    let mut next = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
    next.replace_title(title).unwrap();
    live.replace(next.finish(None).unwrap()).unwrap();
    assert_eq!(
        scope.retained_and_pending_bytes().unwrap(),
        live.snapshot()
            .unwrap()
            .policy_bytes()
            .unwrap()
            .max(original)
    );
    assert_eq!(
        match live.snapshot().unwrap().rows()[0].view() {
            RowBody::Agent(value) => value.read().as_ptr(),
            _ => panic!("agent row"),
        },
        pointer
    );
    let weak = live.snapshot().unwrap().projection();
    reader.retain_source(&live).unwrap();
    for role in [
        &mut live,
        &mut active,
        &mut latest,
        &mut candidate,
        &mut delivery,
    ] {
        role.clear();
    }
    foreground.clear_source();
    // The actual construction source still owns the payload until cleared.
    assert!(scope.retained_and_pending_bytes().unwrap() > crate::history::MAX_RESIDENT_BYTES / 2);
    reader.clear_source();
    let retired = graph::measure_document_usage(&scope, &weak).unwrap();
    assert_eq!(retired.semantic(0).unwrap(), 0);
    assert!(retired.physical(0).unwrap() > 0);
    drop(weak);
    assert!(scope.retained_and_pending_bytes().unwrap() < 1024 * 1024);
}

#[test]
fn snapshot_publication_cannot_occupy_original_prompt_dispatch_capability() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut dispatch = roles.take_prompt_dispatch().unwrap();
    live.replace(snapshot(&reader, "original")).unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    assert!(matches!(
        dispatch.clone_from_role(&live),
        Err(RetentionError::WrongPayload)
    ));
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    assert!(dispatch.snapshot().is_none());
    assert!(matches!(
        dispatch.replace(snapshot(&reader, "unpublishable")),
        Err(RetentionError::WrongPayload)
    ));
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    assert!(dispatch.snapshot().is_none());
    assert_eq!(
        &*live.snapshot().unwrap().id(),
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"
    );
}
