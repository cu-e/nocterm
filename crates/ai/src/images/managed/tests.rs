//! Public managed image construction retains the exact original inspected allocation.
use super::*;
use crate::{
    history::MAX_RESIDENT_BYTES,
    model::{native::NativeBytes, retention::RetentionAuthority},
    testing::{ScriptedImageCodec, images::png_header_fixture},
};
fn bytes(ledger: &DecodeLedger, value: &[u8]) -> NativeOwned<Vec<u8>> {
    let mut bytes = NativeBytes::new(ledger, value.len()).unwrap();
    bytes.append(value).unwrap();
    bytes.finish()
}
fn codec(value: &[u8]) -> ScriptedImageCodec {
    let codec = ScriptedImageCodec::default();
    codec.inspection(value, Ok(super::super::preflight(value).unwrap()));
    codec
}

fn live() -> crate::thread::ThreadState {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    crate::thread::ThreadState::new(
        &authority,
        "agent",
        "00000000-0000-4000-8000-000000000001",
        1,
    )
    .unwrap()
}

#[test]
fn whole_image_future_keeps_encoding_open_until_box_drop_and_installs_after_close() {
    use futures::{FutureExt, task::noop_waker};
    use std::{
        sync::Arc,
        task::{Context, Poll},
    };
    let mut live = live();
    let source = png_header_fixture(2, 2, 33);
    let codec = codec(&source);
    let request = live.live_mut().prepare_image_request(&source).unwrap();
    let mut future = request.inspect(Arc::new(codec.clone())).unwrap();
    let admission = live.live().local_admission();
    live.live_mut()
        .set_draft_at(admission, Some("typing during decode"), 2)
        .unwrap();
    live.prepare_candidate().unwrap();
    let Poll::Ready(image) = future.poll_unpin(&mut Context::from_waker(&noop_waker())) else {
        panic!("synchronous native inspection")
    };
    let image = image.unwrap();
    assert_eq!(image.get().encoded(), STANDARD.encode(&source));
    drop(NativeBytes::new(image.ledger(), 1).unwrap());
    drop(future);
    assert!(matches!(
        NativeBytes::new(image.ledger(), 1),
        Err(JsonDecodeError::Retention(
            crate::model::retention::RetentionError::FrozenOwner
        ))
    ));
    let pointer = image.get().binary().bytes().as_ptr();
    let admission = live.live().local_admission();
    live.live_mut()
        .add_prepared_image(admission, image)
        .unwrap();
    let stored = &live.live().inputs().unwrap().images()[0];
    assert_eq!(stored.binary().bytes().as_ptr(), pointer);
    assert_eq!(stored.binary().bytes(), source);
    assert_eq!(
        live.live().composer().unwrap().draft().as_deref(),
        Some("typing during decode")
    );
    codec.clear_managed_requests();
}

#[test]
fn cancelled_image_future_returns_original_physical_credit_without_changing_live() {
    use std::sync::Arc;
    let mut live = live();
    let scope = live.live().role().scope().clone();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    let revision = live.live().revision();
    let source = png_header_fixture(2, 2, 33);
    let request = live.live_mut().prepare_image_request(&source).unwrap();
    let future = request.inspect(Arc::new(codec(&source))).unwrap();
    assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
    drop(future);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
    assert_eq!(live.live().revision(), revision);
    assert!(live.live().inputs().is_none());
}

#[test]
fn adding_prepared_images_shares_survivors_and_refusal_preserves_original_graph() {
    use futures::executor::block_on;
    use std::sync::Arc;
    let mut live = live();
    let source = png_header_fixture(2, 2, 33);
    let codec = codec(&source);
    let mut first = None;
    for _ in 0..crate::images::MAX_IMAGES {
        let request = live.live_mut().prepare_image_request(&source).unwrap();
        let image = block_on(request.inspect(Arc::new(codec.clone())).unwrap()).unwrap();
        let admission = live.live().local_admission();
        live.live_mut()
            .add_prepared_image(admission, image)
            .unwrap();
        let pointer = live.live().inputs().unwrap().images()[0]
            .binary()
            .bytes()
            .as_ptr();
        assert_eq!(*first.get_or_insert(pointer), pointer);
    }
    let revision = live.live().revision();
    let request = live.live_mut().prepare_image_request(&source).unwrap();
    let image = block_on(request.inspect(Arc::new(codec.clone())).unwrap()).unwrap();
    let admission = live.live().local_admission();
    assert!(matches!(
        live.live_mut().add_prepared_image(admission, image),
        Err(crate::history::live::LiveError::Physical(
            crate::history::physical::PhysicalError::ImageCollection(_)
        ))
    ));
    assert_eq!(live.live().revision(), revision);
    assert_eq!(
        live.live().inputs().unwrap().images().len(),
        crate::images::MAX_IMAGES
    );
    assert_eq!(
        Some(
            live.live().inputs().unwrap().images()[0]
                .binary()
                .bytes()
                .as_ptr()
        ),
        first
    );
    codec.clear_managed_requests();
}
#[test]
fn original_inspected_binary_and_encoded_media_share_prepaid_lifetimes() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let baseline = ledger.live_requested_bytes();
    let source = png_header_fixture(2, 2, 33);
    let codec = codec(&source);
    let image = validate_in(&ledger, bytes(&ledger, &source), &codec).unwrap();
    let requests = codec.managed_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        image.get().binary().bytes().as_ptr() as usize,
        requests[0].0
    );
    assert_eq!(image.get().binary().bytes(), source);
    assert_eq!(image.get().encoded(), STANDARD.encode(&source));
    assert_eq!(image.get().mime(), "image/png");
    assert_eq!(image.get().facts().width(), 2);
    assert_eq!(image.get().facts().height(), 2);
    assert_eq!(
        image.get().encoded_size(),
        crate::history::measure_json(&serde_json::json!({"kind":"media","value":{"kind":"image","encoded":STANDARD.encode(&source),"mime":"image/png","uri":null,"annotations":null,"extensions":null}})).unwrap()
    );
    drop(image);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    codec.clear_managed_requests();
    drop(requests);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn equal_binary_values_keep_distinct_authentic_image_identities() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let source = png_header_fixture(2, 2, 33);
    let codec = codec(&source);
    let first = validate_in(&ledger, bytes(&ledger, &source), &codec).unwrap();
    let second = validate_in(&ledger, bytes(&ledger, &source), &codec).unwrap();
    assert!(!first.get().binary().same_allocation(second.get().binary()));
    assert!(first.get().binary().same_allocation(first.get().binary()));
}
#[test]
fn failed_image_construction_refunds_binary_after_borrowed_inspection() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let baseline = ledger.live_requested_bytes();
    let source = png_header_fixture(2, 2, 33);
    let codec = ScriptedImageCodec::default();
    codec.inspection(&source, Err(super::super::ImageCodecError::InvalidEvidence));
    assert!(validate_in(&ledger, bytes(&ledger, &source), &codec).is_err());
    assert_eq!(codec.managed_requests().len(), 1);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    codec.clear_managed_requests();
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn base64_refusal_after_borrowed_inspection_refunds_every_local() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let source = png_header_fixture(2, 2, 33);
    let supplied = codec(&source);
    let probe = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    drop(validate_in(&probe, bytes(&probe, &source), &supplied).unwrap());
    let limit = probe.peak_requested_bytes() - 1;
    supplied.clear_managed_requests();
    let ledger = DecodeLedger::new(&authority, limit).unwrap();
    let baseline = ledger.live_requested_bytes();
    assert!(validate_in(&ledger, bytes(&ledger, &source), &supplied).is_err());
    assert_eq!(supplied.managed_requests().len(), 1);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    supplied.clear_managed_requests();
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
