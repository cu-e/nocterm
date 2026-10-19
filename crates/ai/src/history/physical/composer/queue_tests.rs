//! Queue edits through builders, registered roles and borrowed physical views.
use crate::{
    history::{
        ComposerData, MAX_RESIDENT_BYTES,
        physical::{ContentSlot, SnapshotBuilder},
    },
    model::{
        content::MediaKind,
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        retention::{
            RetentionAuthority,
            document::{
                AllocationPlan, DocumentRetention,
                tests::{snapshot, text},
            },
        },
    },
};

fn composer(ledger: &DecodeLedger, ids: &[u64]) -> NativeOwned<ComposerData> {
    let mut queue = NativeList::new(ledger, ids.len()).unwrap();
    for &id in ids {
        let mut images = NativeList::new(ledger, 1).unwrap();
        images
            .push(crate::model::native::content::from_media(
                crate::model::native::content::media(
                    MediaKind::Image,
                    text(ledger, "YWJjZA=="),
                    text(ledger, "image/png"),
                    NativeOwned::absent(ledger).unwrap(),
                    NativeOwned::absent(ledger).unwrap(),
                    NativeOwned::absent(ledger).unwrap(),
                )
                .unwrap(),
            ))
            .unwrap();
        let mut attachments = NativeList::new(ledger, 1).unwrap();
        attachments
            .push(crate::history::native::document::connection(text(
                ledger,
                "queued-context",
            )))
            .unwrap();
        queue
            .push(
                crate::history::native::document::prompt(
                    id,
                    text(ledger, "queued text"),
                    images.finish(),
                    attachments.finish(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let mut attachments = NativeList::new(ledger, 1).unwrap();
    attachments
        .push(crate::history::native::document::connection(text(
            ledger,
            "default-context",
        )))
        .unwrap();
    let (draft, allocation) = text(ledger, "keep draft").into_parts();
    crate::history::native::document::composer(
        NativeOwned::from_parts(Some(draft), allocation),
        queue.finish(),
        attachments.finish(),
    )
    .unwrap()
}

fn ids(role: &crate::model::retention::document::RetainedRole) -> Vec<u64> {
    role.snapshot()
        .unwrap()
        .composer()
        .queue()
        .iter()
        .map(|p| p.id())
        .collect()
}

#[test]
fn arbitrary_queue_edits_share_actual_payloads_after_source_retirement() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut writer = roles.take_writer_active().unwrap();
    live.replace(snapshot(&operation, "unchanged transcript"))
        .unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = composer(&ledger, &[11, 22, 33, 44]);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    writer.clone_from_role(&live).unwrap();
    let (text_ptr, images_ptr, attachment_ptr, draft_ptr) = {
        let view = writer.snapshot().unwrap().composer();
        let prompt = &view.queue()[2];
        (
            prompt.text().as_ptr(),
            prompt.images().next().unwrap() as *const ContentSlot,
            prompt.attachments()[0].id().as_ptr(),
            view.draft().unwrap().as_ptr(),
        )
    };
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.remove_prompt(22).unwrap());
    assert!(builder.move_prompt(33, 0).unwrap());
    assert!(builder.move_prompt(11, 2).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    assert_eq!(ids(&live), [33, 44, 11]);
    assert_eq!(ids(&writer), [11, 22, 33, 44]);
    writer.clear();
    let view = live.snapshot().unwrap().composer();
    let prompt = &view.queue()[0];
    assert_eq!(prompt.text().as_ptr(), text_ptr);
    assert_eq!(
        prompt.images().next().unwrap() as *const ContentSlot,
        images_ptr
    );
    assert_eq!(prompt.attachments()[0].id().as_ptr(), attachment_ptr);
    assert_eq!(view.draft().unwrap().as_ptr(), draft_ptr);
    assert_eq!(&*view.draft().unwrap(), "keep draft");
    assert_eq!(&*view.attachments()[0].id(), "default-context");
    let ContentSlot::Media(media) = prompt.images().next().unwrap() else {
        panic!("queued media")
    };
    assert_eq!(&*media.encoded(), "YWJjZA==");
    assert_eq!(&*media.mime(), "image/png");
    assert!(scope.retained_and_pending_bytes().unwrap() > 0);
}

#[test]
fn queue_unknown_equal_and_out_of_range_edits_are_allocation_free_noops() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "unchanged")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = composer(&ledger, &[11, 22]);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    assert!(!builder.remove_prompt(99).unwrap());
    assert!(!builder.move_prompt(99, 0).unwrap());
    assert!(!builder.move_prompt(11, 0).unwrap());
    assert!(!builder.move_prompt(22, 1).unwrap());
    assert!(!builder.move_prompt(11, 2).unwrap());
    assert!(!builder.move_prompt(11, usize::MAX).unwrap());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    live.replace(builder.finish(None).unwrap()).unwrap();
    assert_eq!(ids(&live), [11, 22]);
}

#[test]
fn refused_queue_edits_preserve_the_builders_previously_prepared_state() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let blocker = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "unchanged")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = composer(&ledger, &[11, 22, 33]);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    assert!(builder.move_prompt(33, 0).unwrap());
    let before = scope.retained_and_pending_bytes().unwrap();
    let held = blocker
        .reserve_pending(AllocationPlan::text(MAX_RESIDENT_BYTES - before))
        .unwrap();
    let full = scope.retained_and_pending_bytes().unwrap();
    assert!(!builder.move_prompt(33, 0).unwrap());
    assert!(!builder.remove_prompt(99).unwrap());
    assert!(builder.remove_prompt(22).is_err());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), full);
    assert!(builder.move_prompt(11, 0).is_err());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), full);
    drop(held);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    live.replace(builder.finish(None).unwrap()).unwrap();
    assert_eq!(ids(&live), [33, 11, 22]);
    assert_eq!(
        &*live.snapshot().unwrap().composer().draft().unwrap(),
        "keep draft"
    );
}

#[test]
fn queue_removal_matches_all_original_ids_and_empty_queue_edits_are_noops() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "unchanged")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = composer(&ledger, &[11, 22, 11]);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    assert!(builder.move_prompt(11, 1).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    assert_eq!(ids(&live), [22, 11, 11]);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.remove_prompt(11).unwrap());
    assert!(!builder.move_prompt(22, 0).unwrap());
    assert!(builder.remove_prompt(22).unwrap());
    assert!(!builder.remove_prompt(22).unwrap());
    assert!(!builder.move_prompt(22, 0).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    assert!(ids(&live).is_empty());
}
