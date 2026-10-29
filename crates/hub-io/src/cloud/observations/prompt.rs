//! Borrowed inline content is admitted before native owned strings and image data.
use super::{budget::DecodeBudget, values};
use base64::Engine as _;
use nocterm_ai::model::content::{Content, Image, Text};
use nocterm_cloud_model::{
    coordinator::{ObservedPrompt, RunDestination},
    engine::EnginePrompt,
    projection::ProjectionError,
};
use nocterm_hub_model::cloud::{
    events::AcceptedPrompt,
    payload::{ImageMime, PromptPart},
};

pub(super) fn prompt(
    value: &AcceptedPrompt<'_>,
    budget: &mut DecodeBudget<'_>,
) -> Result<ObservedPrompt, ProjectionError> {
    let slots = value
        .content
        .len()
        .checked_mul(std::mem::size_of::<Content>())
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or(ProjectionError::Capacity)?;
    budget.admit(slots)?; // Vec->Arc transfer retains both slot buffers briefly.
    let mut content = Vec::with_capacity(value.content.len());
    for part in &value.content {
        content.push(match part {
            PromptPart::Text(text) => {
                budget.admit(text.len())?;
                Content::Text(Text::new(*text))
            }
            PromptPart::Image { mime, bytes } => {
                let length =
                    base64::encoded_len(bytes.len(), true).ok_or(ProjectionError::Capacity)?;
                budget.admit(length.checked_add(10).ok_or(ProjectionError::Capacity)?)?;
                let mut data = String::with_capacity(length);
                base64::engine::general_purpose::STANDARD.encode_string(bytes, &mut data);
                Content::Image(Image::new(
                    data,
                    match mime {
                        ImageMime::Png => "image/png",
                        ImageMime::Jpeg => "image/jpeg",
                        ImageMime::Gif => "image/gif",
                        ImageMime::Webp => "image/webp",
                    },
                ))
            }
        });
    }
    let destination = match &value.target {
        Some(access) => {
            let (target, authentication) = values::target(access)?;
            let session = value
                .existing_session
                .as_ref()
                .map(values::session)
                .transpose()?;
            Some(
                RunDestination::new(target, authentication, session)
                    .map_err(|_| ProjectionError::InvalidRecord)?,
            )
        }
        None if value.existing_session.is_none() => None,
        None => return Err(ProjectionError::InvalidRecord),
    };
    Ok(ObservedPrompt::new(
        EnginePrompt::new(content).map_err(|_| ProjectionError::InvalidRecord)?,
        values::model(&value.model)?,
        destination,
    ))
}
