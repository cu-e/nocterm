//! Finite thumbnail work retains original list/row generation and installs into MessageList.
use super::entries::PreviewError;
use super::{
    AgentPanel,
    image_pipeline::{CompletedPreview, PreviewSource, PreviewTicket},
    message_list::{ActiveListBinding, MessageList},
};
use gpui_kit::{Context, Task};
use nocterm_ai::model::content::Content;
pub(super) struct PreviewOperation {
    binding: ActiveListBinding,
    ticket: PreviewTicket,
    row: Option<usize>,
    source: PreviewSource,
    images: crate::ImageDeps,
}
impl PreviewOperation {
    pub(super) fn cached(
        messages: &mut MessageList,
        source: PreviewSource,
        row: Option<usize>,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let key = source.key();
        if let Some(binding) = messages.binding()
            && let Some(ticket) = messages.request_preview(source.clone())
        {
            let task = Self {
                binding,
                ticket,
                row,
                source,
                images: messages.image_deps(),
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
            source,
            images,
        } = self;
        let future = cx.background_executor().spawn(async move {
            let _permit = images.work().await;
            let bytes = source.decode()?;
            let bounds = nocterm_ai::images::PreviewBounds::new(640, 640)?;
            let preview = images.thumbnailer().thumbnail(bytes, bounds)?;
            // Supplied previews retain the same pure size/format contract.
            let preview = nocterm_ai::images::EncodedPreview::new(preview.bytes().clone(), bounds)?;
            Ok::<_, PreviewError>(CompletedPreview::from_encoded(preview))
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
            .map(|image| PreviewSource::Original(image.shared_bytes()))
            .collect::<Vec<_>>();
        if previews.is_empty() {
            return Vec::new();
        }
        previews
            .into_iter()
            .map(|source| Self::cached(messages, source, None, cx))
            .collect()
    }
}

impl PreviewOperation {
    pub(super) fn queued_previews(
        messages: &mut MessageList,
        prompts: &[crate::thread::QueuedPrompt],
        cx: &mut Context<AgentPanel>,
    ) -> Vec<Vec<gpui_kit::AnyElement>> {
        prompts
            .iter()
            .map(|prompt| {
                prompt
                    .saved()
                    .images
                    .iter()
                    .filter_map(|block| {
                        let Content::Image(image) = block else {
                            return None;
                        };
                        Some(Self::cached(
                            messages,
                            PreviewSource::Encoded(image.data.clone()),
                            None,
                            cx,
                        ))
                    })
                    .collect()
            })
            .collect()
    }
}
