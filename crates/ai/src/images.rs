//! Pure prompt admission and injected image inspection/preview contracts.
mod codec;
mod header;
pub mod managed;
use crate::model::content::{Content, Media, MediaKind};
use base64::{Engine, engine::general_purpose::STANDARD};
pub use codec::*;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_IMAGES: usize = 8;
pub const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;
/// Why a candidate prompt image was rejected.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PromptImageError {
    #[error("Images must be at most 5 MiB")]
    TooLarge,
    #[error("Invalid or unsupported image")]
    Unsupported,
    #[error("Image dimensions exceed 16 million pixels")]
    Dimensions,
    #[error("Invalid image header")]
    Header(#[source] ImageCodecError),
    #[error("Invalid image data or decoding limit exceeded")]
    Decode(#[source] ImageCodecError),
    #[error(transparent)]
    Encoding(#[from] crate::history::HistoryCodecError),
}
/// A prompt image collection above the count or total-size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("At most 8 images and 20 MiB total")]
pub struct ImageCollectionError;
#[derive(Clone, Debug)]
pub struct PromptImage {
    mime_type: String,
    data: std::sync::Arc<[u8]>,
    width: u32,
    height: u32,
    content: std::sync::Arc<Content>,
    size: crate::history::FormatSize,
}
impl PromptImage {
    pub fn validate(
        data: Vec<u8>,
        inspector: &dyn ImageInspector,
    ) -> Result<Self, PromptImageError> {
        let facts = preflight(&data).map_err(PromptImageError::from)?;
        let data: std::sync::Arc<[u8]> = data.into();
        let inspected = inspector
            .inspect(data.clone())
            .map_err(PromptImageError::from)?;
        if !std::sync::Arc::ptr_eq(&data, inspected.bytes()) || inspected.facts() != facts {
            return Err(PromptImageError::Decode(ImageCodecError::InvalidEvidence));
        }
        let mime = facts.format().mime();
        let content = Content::Media(Media {
            kind: MediaKind::Image,
            encoded: STANDARD.encode(&data),
            mime: mime.into(),
            uri: None,
            annotations: None,
            extensions: None,
        });
        let size = crate::history::measure_json(&content)
            .map_err(crate::history::HistoryCodecError::from)?;
        Ok(Self {
            mime_type: mime.into(),
            data,
            width: facts.width(),
            height: facts.height(),
            content: std::sync::Arc::new(content),
            size,
        })
    }
    pub fn content(&self) -> Content {
        (*self.content).clone()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn encoded_len(&self) -> usize {
        self.size.encoded_bytes
    }
}
pub fn validate_collection(images: &[PromptImage]) -> Result<(), ImageCollectionError> {
    if images.len() > MAX_IMAGES
        || images.iter().map(|v| v.data.len()).sum::<usize>() > MAX_TOTAL_IMAGE_BYTES
    {
        Err(ImageCollectionError)
    } else {
        Ok(())
    }
}
impl From<ImageCodecError> for PromptImageError {
    fn from(error: ImageCodecError) -> Self {
        match error {
            ImageCodecError::SourceSize => Self::TooLarge,
            ImageCodecError::UnsupportedFormat => Self::Unsupported,
            ImageCodecError::PixelLimit => Self::Dimensions,
            ImageCodecError::InvalidHeader { .. }
            | ImageCodecError::ManagedInvalidHeader { .. } => Self::Header(error),
            _ => Self::Decode(error),
        }
    }
}
#[cfg(test)]
mod tests;
