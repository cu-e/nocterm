//! Finite thumbnail work retains original list/row generation and installs into MessageList.
use super::entries::PreviewError;
use super::{
    AgentPanel,
    image_pipeline::{ImageKey, PreviewTicket},
    message_list::{ActiveListBinding, MessageList},
};
use base64::Engine as _;
use gpui_kit::{Context, Task};
use nocterm_ai::images::ImageThumbnailer;
use std::sync::Arc;
pub(super) struct PreviewOperation {
    binding: ActiveListBinding,
    ticket: PreviewTicket,
    row: Option<usize>,
    bytes: Vec<u8>,
    encoded: bool,
    thumbnailer: Arc<dyn ImageThumbnailer>,
}
impl PreviewOperation {
    pub(super) fn cached(
        messages: &mut MessageList,
        key: ImageKey,
        data: impl FnOnce() -> Vec<u8>,
        encoded: bool,
        row: Option<usize>,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        if let Some(binding) = messages.binding()
            && let Some(ticket) = messages.request_preview(key)
        {
            let task = Self {
                binding,
                ticket,
                row,
                bytes: data(),
                encoded,
                thumbnailer: messages.thumbnailer(),
            }
            .launch(cx);
            messages.install_preview_task(ticket, task);
        }
        super::entries::EntryRenderer::image(messages.preview_state(&key))
    }
    fn launch(self, cx: &mut Context<AgentPanel>) -> Task<()> {
        let Self {
            binding,
            ticket,
            row,
            bytes,
            encoded,
            thumbnailer,
        } = self;
        let future = cx.background_executor().spawn(async move {
            let bytes = if encoded {
                if bytes.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                    return Err(PreviewError::Codec(
                        nocterm_ai::images::ImageCodecError::SourceSize,
                    ));
                }
                base64::engine::general_purpose::STANDARD.decode(bytes)?
            } else {
                bytes
            };
            let bounds = nocterm_ai::images::PreviewBounds::new(640, 640)?;
            let preview = thumbnailer.thumbnail(bytes.into(), bounds)?;
            // Supplied previews retain the same pure size/format contract.
            let preview = nocterm_ai::images::EncodedPreview::new(preview.bytes().clone(), bounds)?;
            Ok::<_, PreviewError>(Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                preview.bytes().to_vec(),
            )))
        });
        cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_preview_completed(binding, ticket, row, result.ok(), cx)
            });
        })
    }
}

impl PreviewOperation {
    pub(super) fn composer_previews(
        messages: &mut MessageList,
        thread: &gpui_kit::Entity<crate::thread::AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<gpui_kit::AnyElement> {
        let previews = thread
            .read(cx)
            .composer()
            .images()
            .iter()
            .map(|image| {
                let binary = image.binary();
                let bytes = binary.bytes();
                let key = (bytes.as_ptr() as usize, bytes.len(), false);
                (key, (!messages.has_preview(&key)).then(|| bytes.to_vec()))
            })
            .collect::<Vec<_>>();
        if previews.is_empty() {
            return Vec::new();
        }
        previews
            .into_iter()
            .map(|(key, data)| {
                Self::cached(messages, key, || data.unwrap_or_default(), false, None, cx)
            })
            .collect()
    }
}

impl PreviewOperation {
    /// The encoded bytes of each queued image whose thumbnail is not yet cached.
    pub(super) fn queued_image_sources(
        messages: &mut MessageList,
        queue: &[nocterm_ai::history::physical::PromptSlot],
    ) -> Vec<Vec<(ImageKey, Option<Vec<u8>>)>> {
        use nocterm_ai::{history::physical::ContentSlot, model::content::MediaKind};
        queue
            .iter()
            .map(|prompt| {
                prompt
                    .images()
                    .filter_map(|block| {
                        let ContentSlot::Media(image) = block else {
                            return None;
                        };
                        if image.kind() != MediaKind::Image {
                            return None;
                        }
                        let encoded = image.encoded();
                        let key = (encoded.as_ptr() as usize, encoded.len(), true);
                        Some((
                            key,
                            (!messages.has_preview(&key)).then(|| encoded.as_bytes().to_vec()),
                        ))
                    })
                    .collect()
            })
            .collect()
    }
    pub(super) fn queued_previews(
        messages: &mut MessageList,
        sources: Vec<Vec<(ImageKey, Option<Vec<u8>>)>>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<Vec<gpui_kit::AnyElement>> {
        sources
            .into_iter()
            .map(|images| {
                images
                    .into_iter()
                    .map(|(key, data)| {
                        Self::cached(messages, key, || data.unwrap_or_default(), true, None, cx)
                    })
                    .collect()
            })
            .collect()
    }
}
