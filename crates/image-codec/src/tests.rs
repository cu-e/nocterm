//! Physical decoder/encoder behavior belongs to the native adapter.
use super::*;
use nocterm_ai::images::{ImageFormat, MAX_IMAGE_BYTES, PromptImage, PromptImageError};
fn encoded(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let pixel = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        width,
        height,
        image::Rgb([31, 53, 71]),
    ));
    let mut output = Cursor::new(Vec::new());
    pixel.write_to(&mut output, format).unwrap();
    output.into_inner()
}
#[test]
fn every_supported_format_is_fully_decoded_and_preserves_prompt_payload() {
    for (format, expected) in [
        (image::ImageFormat::Png, ImageFormat::Png),
        (image::ImageFormat::Jpeg, ImageFormat::Jpeg),
        (image::ImageFormat::Gif, ImageFormat::Gif),
        (image::ImageFormat::WebP, ImageFormat::WebP),
    ] {
        let bytes = encoded(format, 2, 2);
        let image = PromptImage::validate(bytes.clone(), &NativeImageCodec).unwrap();
        assert_eq!(image.bytes(), bytes);
        assert_eq!(image.mime_type(), expected.mime());
        assert_eq!(image.dimensions(), (2, 2));
    }
}
#[test]
fn corrupt_data_header_bombs_and_source_limits_preserve_typed_refusals() {
    use std::error::Error as _;
    let mut bytes = encoded(image::ImageFormat::Png, 2, 2);
    bytes.truncate(33);
    let error = PromptImage::validate(bytes, &NativeImageCodec).unwrap_err();
    assert!(matches!(error, PromptImageError::Decode(_)));
    assert!(error.source().is_some());
    let PromptImageError::Decode(ImageCodecError::InvalidData { source }) = error else {
        panic!("expected native decoding cause");
    };
    assert!(source.downcast_ref::<image::ImageError>().is_some());
    let mut bytes = encoded(image::ImageFormat::Png, 2, 2);
    bytes[16..20].copy_from_slice(&4097u32.to_be_bytes());
    assert!(matches!(
        PromptImage::validate(bytes, &NativeImageCodec),
        Err(PromptImageError::Decode(ImageCodecError::Dimensions))
    ));
    assert!(matches!(
        PromptImage::validate(vec![0; MAX_IMAGE_BYTES + 1], &NativeImageCodec),
        Err(PromptImageError::TooLarge)
    ));
}
#[test]
fn bounded_preview_downsizes_with_aspect_and_preserves_alpha() {
    let mut source = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        800,
        400,
        image::Rgba([12, 34, 56, 77]),
    ))
    .write_to(&mut source, image::ImageFormat::Png)
    .unwrap();
    let bounds = PreviewBounds::new(640, 640).unwrap();
    let preview = NativeImageCodec
        .thumbnail(source.into_inner().into(), bounds)
        .unwrap();
    assert_eq!(preview.dimensions(), (640, 320));
    assert!(preview.bytes().len() <= images::MAX_PREVIEW_BYTES);
    let decoded = image::load_from_memory(preview.bytes()).unwrap().to_rgba8();
    assert_eq!(decoded.get_pixel(100, 100).0[3], 77);
}
#[test]
fn png_sink_refuses_the_first_byte_over_its_limit() {
    let mut sink = PreviewSink::default();
    sink.write_all(&vec![0; images::MAX_PREVIEW_BYTES]).unwrap();
    assert_eq!(
        sink.write_all(&[0]).unwrap_err().kind(),
        std::io::ErrorKind::FileTooLarge
    );
}
#[test]
fn header_dimension_corruption_never_becomes_trusted_evidence() {
    let mut bytes = encoded(image::ImageFormat::Png, 2, 2);
    bytes[16..20].copy_from_slice(&3u32.to_be_bytes());
    assert!(NativeImageCodec.inspect(bytes.into()).is_err());
}
#[test]
fn a_valid_16_bit_png_cannot_allocate_over_the_decoder_budget() {
    let bytes = include_bytes!("fixtures/memory-limit.png");
    assert!(images::preflight(bytes).is_ok());
    let error = NativeImageCodec
        .inspect(Arc::from(bytes.as_slice()))
        .unwrap_err();
    let ImageCodecError::AllocationLimit { source } = error else {
        panic!("expected allocation refusal");
    };
    assert!(source.downcast_ref::<image::ImageError>().is_some());
}
#[test]
fn prompt_image_failures_expose_native_decoding_causes_through_the_public_error_chain() {
    use std::error::Error;
    let mut corrupt = encoded(image::ImageFormat::Png, 2, 2);
    corrupt.truncate(33);
    let allocation = include_bytes!("fixtures/memory-limit.png").to_vec();
    for bytes in [corrupt, allocation] {
        let error = PromptImage::validate(bytes, &NativeImageCodec).unwrap_err();
        let mut source: Option<&(dyn Error + 'static)> = Some(&error);
        let mut found = false;
        while let Some(cause) = source {
            found |= cause.downcast_ref::<image::ImageError>().is_some();
            source = cause.source();
        }
        assert!(
            found,
            "public error chain must expose native ImageError: {error}"
        );
    }
}
