//! Admission and wire mapping of the desktop's captured prompt body.
use nocterm_cloud_conversation_model::{
    CloudConversationError, ConversationAction, ImageMime, ModelName, PromptImage,
};
use nocterm_cloud_device::open::{
    ChatAction, MAX_PROMPT_IMAGE_BYTES, MAX_PROMPT_IMAGE_TOTAL_BYTES, MAX_PROMPT_IMAGES,
    MAX_PROMPT_TEXT_BYTES, ModelChoice,
};
use nocterm_hub_model::cloud::{MAX_ID_BYTES, payload::ImageMime as WireMime};

pub(super) fn bound(action: &mut ConversationAction) -> Result<(), CloudConversationError> {
    let ConversationAction::Prompt {
        text,
        model,
        images,
        ..
    } = action
    else {
        return Ok(());
    };
    if text.len() > MAX_PROMPT_TEXT_BYTES
        || images.len() > MAX_PROMPT_IMAGES
        || images
            .iter()
            .any(|image| image.bytes.is_empty() || image.bytes.len() > MAX_PROMPT_IMAGE_BYTES)
        || images.iter().map(|image| image.bytes.len()).sum::<usize>()
            > MAX_PROMPT_IMAGE_TOTAL_BYTES
    {
        return Err(CloudConversationError::Capacity);
    }
    if text.capacity() > MAX_PROMPT_TEXT_BYTES {
        *text = std::mem::take(text).into_boxed_str().into_string();
    }
    if images.capacity() > MAX_PROMPT_IMAGES {
        *images = std::mem::take(images).into_boxed_slice().into_vec();
    }
    if let Some(model) = model {
        for value in [&mut model.provider, &mut model.model] {
            if value.len() > MAX_ID_BYTES {
                return Err(CloudConversationError::Capacity);
            }
            if value.capacity() > MAX_ID_BYTES {
                *value = std::mem::take(value).into_boxed_str().into_string();
            }
        }
    }
    Ok(())
}

pub(super) fn prompt(
    action: &ConversationAction,
    default: Option<&ModelChoice>,
    first: bool,
) -> Result<ChatAction, CloudConversationError> {
    let ConversationAction::Prompt {
        text,
        server,
        model,
        images,
    } = action
    else {
        return Err(CloudConversationError::Unsupported);
    };
    let selected = match model {
        Some(model) => Some(model_choice(model)),
        None if first => Some(default.cloned().ok_or(CloudConversationError::NoModel)?),
        None => None,
    };
    let server = server.as_ref().map(super::servers::server_id);
    if images.is_empty() {
        Ok(ChatAction::Prompt {
            text: text.clone(),
            model: selected,
            server,
        })
    } else {
        Ok(ChatAction::PromptWithImages {
            text: text.clone(),
            model: selected,
            server,
            images: images.iter().map(image).collect(),
        })
    }
}
fn model_choice(model: &ModelName) -> ModelChoice {
    ModelChoice {
        provider: model.provider.clone(),
        model: model.model.clone(),
    }
}
fn image(image: &PromptImage) -> nocterm_cloud_device::open::PromptImage {
    nocterm_cloud_device::open::PromptImage {
        mime: match image.mime {
            ImageMime::Png => WireMime::Png,
            ImageMime::Jpeg => WireMime::Jpeg,
            ImageMime::Gif => WireMime::Gif,
            ImageMime::Webp => WireMime::Webp,
        },
        bytes: image.bytes.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_conversation_model::ServerKey;
    #[test]
    fn an_explicit_model_and_images_survive_prompts_after_the_first_run() {
        let bytes: std::sync::Arc<[u8]> = [1, 2, 3].as_slice().into();
        let action = ConversationAction::Prompt {
            text: "look".into(),
            server: Some(ServerKey::from_bytes([3; 16])),
            model: Some(ModelName {
                provider: "managed".into(),
                model: "second-model".into(),
            }),
            images: vec![PromptImage {
                mime: ImageMime::Png,
                bytes: bytes.clone(),
            }],
        };
        let mapped = prompt(&action, None, false).unwrap();
        let ChatAction::PromptWithImages {
            text,
            model: Some(model),
            server: Some(server),
            images,
        } = mapped
        else {
            panic!("image prompt")
        };
        assert_eq!(text, "look");
        assert_eq!(model.model, "second-model");
        assert_eq!(server.as_bytes(), &[3; 16]);
        assert!(std::sync::Arc::ptr_eq(&bytes, &images[0].bytes));
    }
    #[test]
    fn empty_image_parts_and_excessive_image_counts_are_refused_before_queueing() {
        let mut action = ConversationAction::Prompt {
            text: "look".into(),
            server: None,
            model: None,
            images: vec![PromptImage {
                mime: ImageMime::Png,
                bytes: [].as_slice().into(),
            }],
        };
        assert!(matches!(
            bound(&mut action),
            Err(CloudConversationError::Capacity)
        ));
        if let ConversationAction::Prompt { images, .. } = &mut action {
            *images = vec![
                PromptImage {
                    mime: ImageMime::Png,
                    bytes: [1].as_slice().into()
                };
                MAX_PROMPT_IMAGES + 1
            ];
        }
        assert!(matches!(
            bound(&mut action),
            Err(CloudConversationError::Capacity)
        ));
    }
}
