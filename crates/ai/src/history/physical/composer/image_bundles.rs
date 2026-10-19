//! Queued media identity, physical retirement, wire preservation and refusal.
use crate::{
    history::{
        ComposerData, MAX_RESIDENT_BYTES, MAX_SAVED_IMAGE_BYTES,
        physical::{ContentSlot, SnapshotBuilder, SnapshotWire},
    },
    model::{
        content::MediaKind,
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        retention::{
            RetentionAuthority,
            document::{
                AllocationPlan, DocumentRetention, RetainedRole,
                tests::{snapshot, text},
            },
        },
    },
};

fn queued_images(ledger: &DecodeLedger, encoded: &str) -> NativeOwned<ComposerData> {
    let mut queue = NativeList::new(ledger, 2).unwrap();
    for id in [7, 9] {
        let mut images = NativeList::new(ledger, 2).unwrap();
        for mime in ["image/png", "image/jpeg"] {
            images
                .push(crate::model::native::content::from_media(
                    crate::model::native::content::media(
                        MediaKind::Image,
                        text(ledger, encoded),
                        text(ledger, mime),
                        NativeOwned::absent(ledger).unwrap(),
                        NativeOwned::absent(ledger).unwrap(),
                        NativeOwned::absent(ledger).unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
        queue
            .push(
                crate::history::native::document::prompt(
                    id,
                    text(ledger, "queued"),
                    images.finish(),
                    NativeList::new(ledger, 0).unwrap().finish(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    crate::history::native::document::composer(
        NativeOwned::absent(ledger).unwrap(),
        queue.finish(),
        NativeList::new(ledger, 0).unwrap().finish(),
    )
    .unwrap()
}

fn image_identities(role: &RetainedRole) -> Vec<(*const ContentSlot, *const u8, *const u8)> {
    role.snapshot().unwrap().composer().queue()[0]
        .images()
        .map(|content| {
            let ContentSlot::Media(media) = content else {
                panic!("queued image")
            };
            (
                content as *const _,
                media.encoded().as_ptr(),
                media.mime().as_ptr(),
            )
        })
        .collect()
}

#[test]
fn queued_image_bundles_survive_reordering_and_last_source_retirement() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut writer = roles.take_writer_active().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&operation, "row")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = queued_images(&ledger, "YWJjZA==");
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    writer.clone_from_role(&live).unwrap();
    let original = image_identities(&writer);
    assert_ne!(original[0].0, original[1].0);
    assert_ne!(original[0].1, original[1].1);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.move_prompt(7, 1).unwrap());
    assert!(builder.remove_prompt(9).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    writer.clear();
    assert_eq!(image_identities(&live), original);
    let images = live.snapshot().unwrap().composer().queue()[0].images();
    assert_eq!(images.len(), 2);
    for image in images {
        let ContentSlot::Media(media) = image else {
            panic!("queued image")
        };
        assert_eq!(&*media.encoded(), "YWJjZA==");
    }
    drop(ledger);
    live.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}

#[test]
fn queued_wire_keeps_complete_large_images_and_matches_saved_prompt_grammar() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "row")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let encoded = "A".repeat(MAX_SAVED_IMAGE_BYTES + 4);
    let value = queued_images(&ledger, &encoded);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let before = serde_json::to_vec(&SnapshotWire::new(live.snapshot().unwrap())).unwrap();
    let wire: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let expected_images: Vec<_> = ["image/png", "image/jpeg"]
        .into_iter()
        .map(|mime| {
            serde_json::json!({
                "kind": "media", "value": {
                    "kind": "image", "encoded": encoded, "mime": mime,
                    "uri": null, "annotations": null, "extensions": null
                }
            })
        })
        .collect();
    let expected = serde_json::json!({
        "draft": null, "attachments": [], "queue": [
            {"id": 7, "text": "queued", "images": expected_images, "attachments": []},
            {"id": 9, "text": "queued", "images": expected_images, "attachments": []}
        ]
    });
    assert_eq!(wire["document"]["composer"], expected);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.move_prompt(7, 1).unwrap());
    assert!(builder.move_prompt(7, 0).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let after = serde_json::to_vec(&SnapshotWire::new(live.snapshot().unwrap())).unwrap();
    assert_eq!(after, before);
}

#[test]
fn refused_image_bundle_construction_preserves_prepared_queue_and_releases_credit() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let blocker = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "row")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = queued_images(&ledger, "YWJjZA==");
    let replacement = queued_images(&ledger, "cmVwbGFjZW1lbnQ=");
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_composer(value).unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    let held = blocker
        .reserve_pending(AllocationPlan::text(MAX_RESIDENT_BYTES - before))
        .unwrap();
    assert!(builder.replace_composer(replacement).is_err());
    drop(held);
    assert!(scope.retained_and_pending_bytes().unwrap() < before);
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let prompt = &live.snapshot().unwrap().composer().queue()[0];
    assert_eq!(prompt.id(), 7);
    assert_eq!(prompt.images().len(), 2);
    for image in prompt.images() {
        let ContentSlot::Media(media) = image else {
            panic!("queued image")
        };
        assert_eq!(&*media.encoded(), "YWJjZA==");
    }
}
