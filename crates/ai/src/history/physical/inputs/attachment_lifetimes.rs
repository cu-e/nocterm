//! Volatile attachment edits exercise genuine registered payload lifetimes.
use crate::{
    history::{
        MAX_RESIDENT_BYTES,
        live::inputs::{LiveAttachment, LocalItemKey},
        physical::{LiveAttachmentSlot, PhysicalError, SnapshotBuilder},
    },
    model::{
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        retention::{
            RetentionAuthority, RetentionError,
            document::{
                AllocationPlan, DocumentRetention,
                tests::{snapshot, text},
            },
        },
    },
};

fn attachments(ledger: &DecodeLedger, name: &str) -> NativeOwned<Vec<LiveAttachment>> {
    let mut list = NativeList::new(ledger, 4).unwrap();
    list.push(crate::history::live::inputs::connection(text(ledger, name)))
        .unwrap();
    list.push(crate::history::live::inputs::group(text(ledger, "group")))
        .unwrap();
    list.push(crate::history::live::inputs::unavailable_local(text(
        ledger, "offline",
    )))
    .unwrap();
    list.push(
        crate::history::live::inputs::local_item(LocalItemKey::from_bits(u64::MAX), ledger)
            .unwrap(),
    )
    .unwrap();
    list.finish()
}

#[test]
fn aliases_keep_real_attachments_alive_and_saved_candidates_keep_persisted_bundles() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut writer = roles.take_writer_active().unwrap();
    let mut candidate = roles.take_candidate().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&operation, "persisted row")).unwrap();
    assert!(live.snapshot().unwrap().live_inputs().is_none());
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = attachments(&ledger, "connection");
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.replace_live_attachments(value).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let attachment_ptr = live
        .snapshot()
        .unwrap()
        .live_inputs()
        .unwrap()
        .attachments()
        .as_ptr();
    writer.clone_from_role(&live).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.prepare_saved_candidate();
    candidate.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let saved = candidate.snapshot().unwrap();
    let original = live.snapshot().unwrap();
    assert!(saved.live_inputs().is_none());
    assert_eq!(saved.id().as_ptr(), original.id().as_ptr());
    assert!(std::ptr::eq(saved.metadata(), original.metadata()));
    assert!(std::ptr::eq(saved.resume(), original.resume()));
    assert_eq!(saved.rows().as_ptr(), original.rows().as_ptr());
    assert_eq!(saved.times().as_ptr(), original.times().as_ptr());
    assert!(std::ptr::eq(saved.composer(), original.composer()));
    live.clear();
    let values = writer
        .snapshot()
        .unwrap()
        .live_inputs()
        .unwrap()
        .attachments();
    assert_eq!(values.as_ptr(), attachment_ptr);
    let LiveAttachmentSlot::Connection(text) = &values[0] else {
        panic!("connection")
    };
    assert_eq!(&*text.read(), "connection");
    let LiveAttachmentSlot::LocalItem(key) = &values[3] else {
        panic!("local key")
    };
    assert_eq!(key.bits(), u64::MAX);
    let before = scope.retained_and_pending_bytes().unwrap();
    writer.clear();
    assert!(scope.retained_and_pending_bytes().unwrap() < before);
    candidate.clear();
    drop(ledger);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}

#[test]
fn equality_and_refused_or_foreign_replacements_preserve_prepared_builder() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let blocker = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "persisted")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = attachments(&ledger, "first");
    let equal = attachments(&ledger, "first");
    let replacement = attachments(&ledger, "replacement");
    let (_foreign_scope, mut foreign_roles) = DocumentRetention::new(&authority).unwrap();
    let foreign_operation = foreign_roles.take_foreground().unwrap();
    let foreign_ledger =
        DecodeLedger::for_operation(&foreign_operation, MAX_RESIDENT_BYTES).unwrap();
    let foreign = attachments(&foreign_ledger, "first");
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.replace_live_attachments(value).unwrap());
    assert!(!builder.replace_live_attachments(equal).unwrap());
    assert!(matches!(
        builder.replace_live_attachments(foreign),
        Err(PhysicalError::Retention(RetentionError::ForeignAuthority))
    ));
    let before = scope.retained_and_pending_bytes().unwrap();
    let held = blocker
        .reserve_pending(AllocationPlan::text(MAX_RESIDENT_BYTES - before))
        .unwrap();
    assert!(builder.replace_live_attachments(replacement).is_err());
    drop(held);
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let values = live
        .snapshot()
        .unwrap()
        .live_inputs()
        .unwrap()
        .attachments();
    let LiveAttachmentSlot::Connection(text) = &values[0] else {
        panic!("connection")
    };
    assert_eq!(&*text.read(), "first");
}
