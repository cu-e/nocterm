//! Prepaid managed prompt images; native decoding remains an injected operation.
mod binary;
mod operation;
pub use operation::{ImageOperationError, ManagedImageFuture, ManagedImageRequest};
#[cfg(test)]
mod tests;
use super::{ImageFacts, ImageInspector, PromptImageError};
use crate::{
    history::FormatSize,
    model::{
        content::{Content, Media, MediaKind},
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::NativeOwned,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
pub(crate) use binary::BinaryProjection;
pub use binary::ManagedImageBytes;

/// The genuine inspected binary and encoded media before one physical install.
/// This owner is not a second live document or a copy of existing content.
pub struct PreparedPromptImage {
    binary: ManagedImageBytes,
    media: Media,
    facts: ImageFacts,
    size: FormatSize,
}
impl PreparedPromptImage {
    pub fn binary(&self) -> &ManagedImageBytes {
        &self.binary
    }
    pub fn encoded(&self) -> &str {
        &self.media.encoded
    }
    pub fn mime(&self) -> &str {
        &self.media.mime
    }
    pub fn facts(&self) -> ImageFacts {
        self.facts
    }
    pub fn encoded_size(&self) -> FormatSize {
        self.size
    }
    pub(crate) fn into_parts(self) -> (ManagedImageBytes, Media, ImageFacts, FormatSize) {
        (self.binary, self.media, self.facts, self.size)
    }
}
/// Inspect the exact prepaid Arc that will be retained by the resulting image.
/// The original ledger remains alive throughout this native operation. Decoder
/// and thumbnail temporary allocations still need their native admission hooks.
pub fn validate_in(
    ledger: &DecodeLedger,
    data: NativeOwned<Vec<u8>>,
    inspector: &dyn ImageInspector,
) -> Result<NativeOwned<PreparedPromptImage>, PromptImageError> {
    validate_owned(ledger, data, inspector).map_err(ImageOperationError::into_prompt)
}
fn validate_owned(
    ledger: &DecodeLedger,
    data: NativeOwned<Vec<u8>>,
    inspector: &dyn ImageInspector,
) -> Result<NativeOwned<PreparedPromptImage>, ImageOperationError> {
    let facts = super::preflight(data.get())?;
    let binary = ManagedImageBytes::from_native(ledger, data)?;
    let inspected = inspector.inspect_managed(&binary, ledger)?;
    if inspected != facts {
        return Err(super::ImageCodecError::InvalidEvidence.into());
    }
    let encoded_bytes =
        base64::encoded_len(binary.bytes().len(), true).ok_or(JsonDecodeError::Arithmetic)?;
    let mime = facts.format().mime();
    let allocation = ledger.reserve(
        encoded_bytes
            .checked_add(mime.len())
            .ok_or(JsonDecodeError::Arithmetic)?,
    )?;
    let mut encoded = exact_text(encoded_bytes)?;
    STANDARD.encode_string(binary.bytes(), &mut encoded);
    let mut mime_type = exact_text(mime.len())?;
    mime_type.push_str(mime);
    let content = Content::Media(Media {
        kind: MediaKind::Image,
        encoded,
        mime: mime_type,
        uri: None,
        annotations: None,
        extensions: None,
    });
    let size = crate::history::measure_json(&content)?;
    let Content::Media(media) = content else {
        unreachable!("actual image variant")
    };
    Ok(NativeOwned::from_parts(
        PreparedPromptImage {
            binary,
            media,
            facts,
            size,
        },
        allocation,
    ))
}
fn exact_text(capacity: usize) -> Result<String, JsonDecodeError> {
    let mut value = String::new();
    value.try_reserve_exact(capacity)?;
    if value.capacity() != capacity {
        return Err(JsonDecodeError::CapacityProfile);
    }
    Ok(value)
}
