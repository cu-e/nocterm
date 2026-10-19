//! Bounded native image decoding and PNG previews over AI's pure contracts.
//!
//! Owns: PNG/JPEG/GIF/WebP decoder limits, physical validation and preview encoding.
//! Does not own: workers, filesystem access, prompt admission or UI/cache lifecycle.
//! Ports provided: ImageInspector and ImageThumbnailer; ports used: none.
//! Settings section: none. Globals: none.
//! Invariants: source preflight precedes allocation; decode is capped at 4096 and
//! 64 MiB; PNG previews stay within requested dimensions, 640 pixels and 4 MiB.
use image::{GenericImageView as _, ImageEncoder as _};
use nocterm_ai::images::{
    self, EncodedPreview, ImageCodecError, ImageFacts, ImageInspector, ImageThumbnailer,
    InspectedImage, PreviewBounds,
};
use std::{
    io::{Cursor, Write},
    sync::Arc,
};
#[derive(Clone, Copy, Default)]
pub struct NativeImageCodec;
impl NativeImageCodec {
    fn decode(bytes: &[u8]) -> Result<(image::DynamicImage, ImageFacts), ImageCodecError> {
        let facts = images::preflight(bytes)?;
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|source| ImageCodecError::InvalidHeader {
                source: images::ImageCodecCause::new(source),
            })?;
        let expected = match facts.format() {
            images::ImageFormat::Png => image::ImageFormat::Png,
            images::ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            images::ImageFormat::Gif => image::ImageFormat::Gif,
            images::ImageFormat::WebP => image::ImageFormat::WebP,
        };
        if reader.format() != Some(expected) {
            return Err(ImageCodecError::InvalidEvidence);
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(images::MAX_DECODE_DIMENSION);
        limits.max_image_height = Some(images::MAX_DECODE_DIMENSION);
        limits.max_alloc = Some(images::MAX_DECODE_BYTES);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|source| {
            if matches!(source, image::ImageError::Limits(_)) {
                ImageCodecError::AllocationLimit {
                    source: images::ImageCodecCause::new(source),
                }
            } else {
                ImageCodecError::InvalidData {
                    source: images::ImageCodecCause::new(source),
                }
            }
        })?;
        if decoded.dimensions() != (facts.width(), facts.height()) {
            return Err(ImageCodecError::InvalidEvidence);
        }
        Ok((decoded, facts))
    }
}
impl ImageInspector for NativeImageCodec {
    fn inspect(&self, bytes: Arc<[u8]>) -> Result<InspectedImage, ImageCodecError> {
        let (_, facts) = Self::decode(&bytes)?;
        InspectedImage::new(bytes, facts)
    }
}
impl ImageThumbnailer for NativeImageCodec {
    fn thumbnail(
        &self,
        bytes: Arc<[u8]>,
        bounds: PreviewBounds,
    ) -> Result<EncodedPreview, ImageCodecError> {
        let (image, _) = Self::decode(&bytes)?;
        let thumbnail = image.thumbnail(bounds.width(), bounds.height()).to_rgba8();
        let mut sink = PreviewSink::default();
        let result = image::codecs::png::PngEncoder::new(&mut sink).write_image(
            thumbnail.as_raw(),
            thumbnail.width(),
            thumbnail.height(),
            image::ExtendedColorType::Rgba8,
        );
        if sink.exceeded {
            return Err(ImageCodecError::PreviewOutputLimit);
        }
        result.map_err(|source| ImageCodecError::Encode {
            source: images::ImageCodecCause::new(source),
        })?;
        EncodedPreview::new(sink.bytes.into(), bounds)
    }
}
#[derive(Default)]
struct PreviewSink {
    bytes: Vec<u8>,
    exceeded: bool,
}
impl Write for PreviewSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > images::MAX_PREVIEW_BYTES.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "PNG preview output limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
mod tests;
