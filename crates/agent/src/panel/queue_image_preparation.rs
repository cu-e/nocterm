//! Queue image reconstruction preserves original prompt, composer and session identity.
use super::{AgentPanel, composer_view::ComposerBinding, images::ImageAttachError};
use crate::thread::AgentThread;
use gpui_kit::{Context, Task, WeakEntity};
use nocterm_ai::{
    acp,
    images::{ImageInspector, PromptImage},
    session::Ticket,
};
use std::sync::Arc;
pub(super) struct QueueImagePreparation {
    binding: ComposerBinding,
    thread: WeakEntity<AgentThread>,
    ticket: Ticket,
    prompt_id: u64,
    saved: Arc<nocterm_ai::history::SavedPrompt>,
    inspector: Arc<dyn ImageInspector>,
}
impl QueueImagePreparation {
    pub(super) fn new(
        binding: ComposerBinding,
        thread: WeakEntity<AgentThread>,
        ticket: Ticket,
        prompt_id: u64,
        saved: Arc<nocterm_ai::history::SavedPrompt>,
        inspector: Arc<dyn ImageInspector>,
    ) -> Self {
        Self {
            binding,
            thread,
            ticket,
            prompt_id,
            saved,
            inspector,
        }
    }
    pub(super) fn launch(self, cx: &mut Context<AgentPanel>) -> Task<()> {
        let Self {
            binding,
            thread,
            ticket,
            prompt_id,
            saved,
            inspector,
        } = self;
        let future = cx.background_executor().spawn(async move {
            saved
                .images
                .iter()
                .filter_map(|block| match block {
                    acp::ContentBlock::Image(image) => Some(
                        super::queue::decode_saved_image(&image.data).and_then(|bytes| {
                            PromptImage::validate(bytes, inspector.as_ref())
                                .map_err(ImageAttachError::from)
                        }),
                    ),
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()
        });
        cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_queue_image_prepared(binding, prompt_id, thread, ticket, result, cx)
            });
        })
    }
}
