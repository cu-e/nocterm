//! Inline images use an admitted decoded generation; text remains borrowed.
use super::{encoding_error, values};
use base64::Engine as _;
use nocterm_ai::model::content::Content;
use nocterm_cloud_model::{
    coordinator::PromptRequest,
    journal::{JournalError, PublicationAllocation, PublicationWorkBudget},
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::{
    events::AcceptedPrompt,
    payload::{ImageMime, PromptPart},
};

#[derive(Debug, thiserror::Error)]
enum PromptMappingError {
    #[error("the Cloud prompt contains unsupported inline content")]
    Content,
    #[error("the Cloud prompt image has an unsupported media type")]
    Image,
}
struct DecodedImage {
    bytes: Vec<u8>,
    mime: ImageMime,
    _allocation: AllocationLease,
}
impl DecodedImage {
    fn new(
        image: &nocterm_ai::model::content::Image,
        work: &dyn PublicationWorkBudget,
    ) -> Result<Self, JournalError> {
        if image.uri.is_some() {
            return Err(encoding_error(PromptMappingError::Content));
        }
        let mime = match image.mime_type.as_str() {
            "image/png" => ImageMime::Png,
            "image/jpeg" => ImageMime::Jpeg,
            "image/gif" => ImageMime::Gif,
            "image/webp" => ImageMime::Webp,
            _ => return Err(encoding_error(PromptMappingError::Image)),
        };
        let maximum = base64::decoded_len_estimate(image.data.len());
        if maximum > nocterm_hub_model::cloud::MAX_PLAINTEXT_BYTES {
            return Err(JournalError::Capacity);
        }
        let allocation = work.try_reserve(PublicationAllocation::new(maximum.max(1), 1)?)?;
        let mut bytes = Vec::with_capacity(maximum);
        base64::engine::general_purpose::STANDARD
            .decode_vec(&image.data, &mut bytes)
            .map_err(encoding_error)?;
        Ok(Self {
            bytes,
            mime,
            _allocation: allocation,
        })
    }
}

pub(super) fn with_prompt<T>(
    request: &PromptRequest,
    work: &dyn PublicationWorkBudget,
    emit: impl for<'a> FnOnce(AcceptedPrompt<'a>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    let parts = request.prompt().content();
    if parts.is_empty() || parts.len() > 128 {
        return Err(JournalError::Capacity);
    }
    let count = parts
        .iter()
        .filter(|part| matches!(part, Content::Image(_)))
        .count();
    let array_bytes = count
        .checked_mul(std::mem::size_of::<DecodedImage>())
        .ok_or(JournalError::Capacity)?;
    let _array = if array_bytes == 0 {
        None
    } else {
        Some(work.try_reserve(PublicationAllocation::new(array_bytes, 1)?)?)
    };
    let mut images = Vec::with_capacity(count);
    for part in parts {
        if let Content::Image(image) = part {
            images.push(DecodedImage::new(image, work)?);
        }
    }
    let mut decoded = images.iter();
    let mut content = Vec::with_capacity(parts.len());
    for part in parts {
        content.push(match part {
            Content::Text(text) => PromptPart::Text(&text.text),
            Content::Image(_) => {
                let image = decoded.next().ok_or(JournalError::Corrupt)?;
                PromptPart::Image {
                    mime: image.mime,
                    bytes: &image.bytes,
                }
            }
            _ => return Err(encoding_error(PromptMappingError::Content)),
        });
    }
    let destination = request.destination();
    let prompt = AcceptedPrompt {
        content,
        model: values::model(request.selection())?,
        target: Some(values::target(
            destination.target(),
            destination.authentication(),
        )?),
        existing_session: destination.session().map(values::session).transpose()?,
    };
    emit(prompt)
}
