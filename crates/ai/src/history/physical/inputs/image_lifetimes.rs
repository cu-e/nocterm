//! Registered live/input transitions keep actual binary and media owners alive.
use crate::{
    history::{MAX_RESIDENT_BYTES, physical::SnapshotBuilder},
    images::managed::{PreparedPromptImage, validate_in},
    model::{
        json::deferred::DecodeLedger,
        native::{NativeBytes, NativeList, NativeOwned},
        retention::{
            RetentionAuthority,
            document::{
                DocumentRetention,
                tests::{snapshot, text},
            },
        },
    },
    testing::{ScriptedImageCodec, images::png_header_fixture},
};
fn images(ledger: &DecodeLedger, marker: u8) -> NativeOwned<Vec<PreparedPromptImage>> {
    let mut bytes = png_header_fixture(2, 2, 33);
    bytes[32] = marker;
    let codec = ScriptedImageCodec::default();
    codec.inspection(&bytes, Ok(crate::images::preflight(&bytes).unwrap()));
    let mut owned = NativeBytes::new(ledger, bytes.len()).unwrap();
    owned.append(&bytes).unwrap();
    let image = validate_in(ledger, owned.finish(), &codec).unwrap();
    codec.clear_managed_requests();
    let mut images = NativeList::new(ledger, 1).unwrap();
    images.push(image).unwrap();
    images.finish()
}
#[test]
fn attachment_replacement_and_old_writer_keep_original_binary_without_copying() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut old_writer = roles.take_writer_active().unwrap();
    let mut candidate = roles.take_candidate().unwrap();
    live.replace(snapshot(&operation, "saved row")).unwrap();
    let ledger = DecodeLedger::for_operation(&operation, MAX_RESIDENT_BYTES).unwrap();
    let value = images(&ledger, 1);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_images(value).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    old_writer.clone_from_role(&live).unwrap();
    let mut attachments = NativeList::new(&ledger, 1).unwrap();
    attachments
        .push(crate::history::live::inputs::group(text(&ledger, "group")))
        .unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder
        .replace_live_attachments(attachments.finish())
        .unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    assert!(
        image(&live)
            .binary()
            .same_allocation(&image(&old_writer).binary())
    );
    let replacement = images(&ledger, 2);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.replace_images(replacement).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    assert_eq!(image(&old_writer).binary().bytes()[32], 1);
    assert_eq!(image(&live).binary().bytes()[32], 2);
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.prepare_saved_candidate();
    candidate.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    assert!(candidate.snapshot().unwrap().live_inputs().is_none());
    assert_eq!(candidate.snapshot().unwrap().rows().len(), 1);
    live.clear();
    assert_eq!(image(&old_writer).binary().bytes()[32], 1);
}

fn image(
    role: &crate::model::retention::document::RetainedRole,
) -> &crate::history::physical::ManagedImage {
    &role.snapshot().unwrap().live_inputs().unwrap().images()[0]
}
