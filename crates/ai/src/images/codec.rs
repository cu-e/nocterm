//! Immutable codec evidence and allocation contracts; no decoding or native I/O.
use super::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, header};
use std::sync::Arc;
pub const MAX_DECODE_DIMENSION: u32 = 4096;
pub const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PREVIEW_DIMENSION: u32 = 640;
pub const MAX_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
}
impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
        }
    }
}
/// Shared native cause whose source chain reaches the original concrete error.
pub use nocterm_core::SharedCause as ImageCodecCause;
#[derive(Clone, Debug, thiserror::Error)]
pub enum ImageCodecError {
    #[error("Image source is empty or exceeds 5 MiB")]
    SourceSize,
    #[error("Invalid or unsupported image")]
    UnsupportedFormat,
    #[error("Invalid image header: {source}")]
    InvalidHeader {
        #[source]
        source: ImageCodecCause,
    },
    #[error("Image dimensions exceed 16 million pixels")]
    PixelLimit,
    #[error("Image dimensions exceed the decoding limit")]
    Dimensions,
    #[error("Image allocation exceeds the decoding limit")]
    AllocationLimit {
        #[source]
        source: ImageCodecCause,
    },
    #[error("Invalid image data: {source}")]
    InvalidData {
        #[source]
        source: ImageCodecCause,
    },
    #[error("Image codec returned inconsistent evidence")]
    InvalidEvidence,
    #[error("Invalid preview bounds")]
    InvalidPreviewBounds,
    #[error("Encoded preview exceeds 4 MiB")]
    PreviewOutputLimit,
    #[error("Could not encode preview: {source}")]
    Encode {
        #[source]
        source: ImageCodecCause,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageFacts {
    format: ImageFormat,
    width: u32,
    height: u32,
}
impl ImageFacts {
    pub fn new(format: ImageFormat, width: u32, height: u32) -> Result<Self, ImageCodecError> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(ImageCodecError::PixelLimit);
        }
        if width > MAX_DECODE_DIMENSION || height > MAX_DECODE_DIMENSION {
            return Err(ImageCodecError::Dimensions);
        }
        Ok(Self {
            format,
            width,
            height,
        })
    }
    pub fn format(self) -> ImageFormat {
        self.format
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
}
pub fn preflight(bytes: &[u8]) -> Result<ImageFacts, ImageCodecError> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageCodecError::SourceSize);
    }
    let (format, width, height) =
        header::dimensions(bytes).ok_or(ImageCodecError::UnsupportedFormat)?;
    ImageFacts::new(format, width, height)
}
#[derive(Clone, Debug)]
pub struct InspectedImage {
    bytes: Arc<[u8]>,
    facts: ImageFacts,
}
impl InspectedImage {
    pub fn new(bytes: Arc<[u8]>, facts: ImageFacts) -> Result<Self, ImageCodecError> {
        if preflight(&bytes)? != facts {
            return Err(ImageCodecError::InvalidEvidence);
        }
        Ok(Self { bytes, facts })
    }
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
    pub fn facts(&self) -> ImageFacts {
        self.facts
    }
}
pub trait ImageInspector: Send + Sync {
    fn inspect(&self, bytes: Arc<[u8]>) -> Result<InspectedImage, ImageCodecError>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewBounds {
    width: u32,
    height: u32,
}
impl PreviewBounds {
    pub fn new(width: u32, height: u32) -> Result<Self, ImageCodecError> {
        if width == 0
            || height == 0
            || width > MAX_PREVIEW_DIMENSION
            || height > MAX_PREVIEW_DIMENSION
        {
            return Err(ImageCodecError::InvalidPreviewBounds);
        }
        Ok(Self { width, height })
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
}
#[derive(Clone, Debug)]
pub struct EncodedPreview {
    bytes: Arc<[u8]>,
    width: u32,
    height: u32,
}
impl EncodedPreview {
    pub fn new(bytes: Arc<[u8]>, bounds: PreviewBounds) -> Result<Self, ImageCodecError> {
        if bytes.len() > MAX_PREVIEW_BYTES {
            return Err(ImageCodecError::PreviewOutputLimit);
        }
        let facts = preflight(&bytes)?;
        if facts.format() != ImageFormat::Png
            || facts.width() > bounds.width()
            || facts.height() > bounds.height()
        {
            return Err(ImageCodecError::InvalidEvidence);
        }
        Ok(Self {
            bytes,
            width: facts.width(),
            height: facts.height(),
        })
    }
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
pub trait ImageThumbnailer: Send + Sync {
    fn thumbnail(
        &self,
        bytes: Arc<[u8]>,
        bounds: PreviewBounds,
    ) -> Result<EncodedPreview, ImageCodecError>;
}
