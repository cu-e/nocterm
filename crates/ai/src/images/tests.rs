//! Pure admission rejects before inspection and preserves original encoded payload.
use super::*;
use crate::model::content::Content;
use crate::testing::{ScriptedImageCodec, images::png_header_fixture};
use std::sync::Arc;
fn codec(bytes: &[u8]) -> ScriptedImageCodec {
    let codec = ScriptedImageCodec::default();
    codec.inspection(bytes, Ok(preflight(bytes).unwrap()));
    codec
}
#[test]
fn invalid_candidates_never_reach_the_inspector() {
    let codec = ScriptedImageCodec::default();
    for (bytes, message) in [
        (vec![], "Images must be at most 5 MiB"),
        (vec![0; MAX_IMAGE_BYTES + 1], "Images must be at most 5 MiB"),
        (
            b"\x89PNG\r\n\x1a\n".to_vec(),
            "Invalid or unsupported image",
        ),
        (
            png_header_fixture(0, 1, 33),
            "Image dimensions exceed 16 million pixels",
        ),
        (
            png_header_fixture(4097, 1, 33),
            "Invalid image data or decoding limit exceeded",
        ),
        (
            png_header_fixture(100_000, 100_000, 33),
            "Image dimensions exceed 16 million pixels",
        ),
    ] {
        assert_eq!(
            PromptImage::validate(bytes, &codec)
                .unwrap_err()
                .to_string(),
            message
        );
    }
    assert!(codec.requests().is_empty());
}
#[test]
fn supplied_evidence_keeps_original_bytes_mime_and_exact_history_budget() {
    let bytes = png_header_fixture(2, 2, 33);
    let codec = codec(&bytes);
    let image = PromptImage::validate(bytes.clone(), &codec).unwrap();
    assert_eq!(image.bytes(), bytes);
    assert_eq!(image.mime_type(), "image/png");
    assert_eq!(image.dimensions(), (2, 2));
    assert_eq!(
        image.encoded_len(),
        crate::history::content_size(&image.content()).unwrap()
    );
    let Content::Image(content) = image.content() else {
        panic!("expected image");
    };
    assert_eq!(content.data, STANDARD.encode(bytes));
    assert_eq!(
        validate_collection(&vec![image; 9]),
        Err(ImageCollectionError)
    );
}
#[test]
fn checked_images_retain_exact_aggregate_admission_without_a_bypass_constructor() {
    let bytes = png_header_fixture(1, 1, MAX_IMAGE_BYTES);
    let image = PromptImage::validate(bytes.clone(), &codec(&bytes)).unwrap();
    assert!(validate_collection(&vec![image.clone(); 4]).is_ok());
    assert_eq!(
        validate_collection(&vec![image; 5]),
        Err(ImageCollectionError)
    );
}
#[test]
fn inconsistent_dimensions_and_replaced_source_are_refused() {
    let bytes = png_header_fixture(2, 2, 33);
    let codec = ScriptedImageCodec::default();
    codec.inspection(&bytes, Ok(ImageFacts::new(ImageFormat::Png, 1, 1).unwrap()));
    assert!(matches!(
        PromptImage::validate(bytes.clone(), &codec),
        Err(PromptImageError::Decode(ImageCodecError::InvalidEvidence))
    ));
    let codec = codec_with_replacement(&bytes);
    assert!(matches!(
        PromptImage::validate(bytes, &codec),
        Err(PromptImageError::Decode(ImageCodecError::InvalidEvidence))
    ));
}
fn codec_with_replacement(bytes: &[u8]) -> ScriptedImageCodec {
    let codec = codec(bytes);
    codec.replace_source(bytes);
    codec
}
#[test]
fn preview_contract_rejects_oversized_output_format_and_bounds() {
    assert!(PreviewBounds::new(0, 1).is_err());
    assert!(PreviewBounds::new(641, 640).is_err());
    let bounds = PreviewBounds::new(640, 640).unwrap();
    assert!(matches!(
        EncodedPreview::new(Arc::from(vec![0; MAX_PREVIEW_BYTES + 1]), bounds),
        Err(ImageCodecError::PreviewOutputLimit)
    ));
    assert!(EncodedPreview::new(png_header_fixture(641, 1, 33).into(), bounds).is_err());
}
#[test]
fn all_header_formats_and_jpeg_scan_boundaries_remain_pure() {
    let png = png_header_fixture(2, 3, 33);
    let mut gif = b"GIF89a\x02\x00\x03\x00\x00\x00\x00".to_vec();
    let jpeg = vec![0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 3, 0, 2, 0];
    let mut webp = vec![0; 30];
    webp[..4].copy_from_slice(b"RIFF");
    webp[8..12].copy_from_slice(b"WEBP");
    webp[12..16].copy_from_slice(b"VP8X");
    webp[24] = 1;
    webp[27] = 2;
    for (bytes, format) in [
        (png, ImageFormat::Png),
        (gif.clone(), ImageFormat::Gif),
        (jpeg.clone(), ImageFormat::Jpeg),
        (webp, ImageFormat::WebP),
    ] {
        let facts = preflight(&bytes).unwrap();
        assert_eq!(facts.format(), format);
        assert_eq!((facts.width(), facts.height()), (2, 3));
    }
    gif.truncate(12);
    assert!(preflight(&gif).is_err());
    for end in 0..jpeg.len() {
        assert!(preflight(&jpeg[..end]).is_err());
    }
    assert!(preflight(&[0xff, 0xd8, 0xff, 0xc0, 0, 1]).is_err());
}
#[test]
fn a_supplied_header_failure_keeps_its_typed_io_cause_in_the_public_chain() {
    use std::error::Error;
    let bytes = png_header_fixture(2, 2, 33);
    let codec = ScriptedImageCodec::default();
    codec.inspection(
        &bytes,
        Err(ImageCodecError::InvalidHeader {
            source: ImageCodecCause::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "supplied header failure",
            )),
        }),
    );
    let error = PromptImage::validate(bytes, &codec).unwrap_err();
    assert_eq!(error.to_string(), "Invalid image header");
    let mut source: Option<&(dyn Error + 'static)> = Some(&error);
    let mut found = false;
    while let Some(cause) = source {
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            found = true;
        }
        source = cause.source();
    }
    assert!(found);
}
