//! Managed PNG uses original source ownership, grammar and native error causes.
use super::*;
use crate::NativeImageCodec;
use nocterm_ai::{
    history::MAX_RESIDENT_BYTES,
    images::{ImageInspector, PromptImageError, managed::validate_in},
    model::{native::NativeBytes, retention::RetentionAuthority},
};
use std::{error::Error, io::Cursor};

fn input(ledger: &DecodeLedger, bytes: &[u8]) -> nocterm_ai::model::native::NativeOwned<Vec<u8>> {
    let mut input = NativeBytes::new(ledger, bytes.len()).unwrap();
    input.append(bytes).unwrap();
    input.finish()
}
fn encoded(image: image::DynamicImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}
#[test]
fn managed_png_matches_original_complete_decode_and_preserves_original_payload() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let baseline = ledger.live_requested_bytes();
    for image in [
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([2, 3, 4]))),
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            2,
            image::Rgba([2, 3, 4, 5]),
        )),
        image::DynamicImage::ImageLuma16(image::ImageBuffer::from_pixel(3, 2, image::Luma([1025]))),
    ] {
        let bytes = encoded(image);
        let original = NativeImageCodec.inspect(bytes.clone().into()).unwrap();
        let managed = validate_in(&ledger, input(&ledger, &bytes), &NativeImageCodec).unwrap();
        assert_eq!(managed.get().facts(), original.facts());
        assert_eq!(managed.get().binary().bytes(), bytes);
        assert_eq!(managed.get().mime(), "image/png");
        drop(managed);
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
#[test]
fn managed_corrupt_png_keeps_original_native_error_through_last_clone_drop() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut bytes = encoded(image::DynamicImage::new_rgb8(2, 2));
    bytes.truncate(33);
    let original_error = NativeImageCodec.inspect(bytes.clone().into()).unwrap_err();
    let ImageCodecError::InvalidData { source: original } = &original_error else {
        panic!("original native data category: {original_error}");
    };
    assert!(original.source().unwrap().is::<image::ImageError>());
    let error = validate_in(&ledger, input(&ledger, &bytes), &NativeImageCodec)
        .err()
        .unwrap();
    let PromptImageError::Decode(managed @ ImageCodecError::ManagedInvalidData { source }) = &error
    else {
        panic!("managed native data category: {error}");
    };
    let mut cause: Option<&(dyn Error + 'static)> = Some(source);
    let mut native = None;
    while let Some(current) = cause {
        if let Some(original) = current.downcast_ref::<image::ImageError>() {
            native = Some(original as *const _);
        }
        cause = current.source();
    }
    let pointer = native.expect("original concrete native ImageError");
    assert_eq!(managed.to_string(), original_error.to_string());
    let held = ledger.live_requested_bytes();
    assert!(held > baseline);
    let cloned = error.clone();
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), held);
    let mut cause: Option<&(dyn Error + 'static)> = Some(&cloned);
    let mut same = false;
    while let Some(current) = cause {
        same |= current
            .downcast_ref::<image::ImageError>()
            .is_some_and(|value| std::ptr::eq(value, pointer));
        cause = current.source();
    }
    assert!(same);
    drop(cloned);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
#[test]
fn managed_png_preserves_existing_native_output_limit_without_blanket_reservation() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let baseline = ledger.live_requested_bytes();
    let bytes = include_bytes!("../fixtures/memory-limit.png");
    let error = validate_in(&ledger, input(&ledger, bytes), &NativeImageCodec)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        PromptImageError::Decode(ImageCodecError::ManagedAllocationLimit { .. })
    ));
    drop(error);
    assert_eq!(ledger.live_requested_bytes(), baseline);
    let small = encoded(image::DynamicImage::new_rgb8(2, 2));
    drop(validate_in(&ledger, input(&ledger, &small), &NativeImageCodec).unwrap());
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
