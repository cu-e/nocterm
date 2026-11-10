//! Queue image reconstruction preserves original prompt, composer and session identity.
use super::{AgentPanel, composer_view::ComposerBinding, images::ImageAttachError};
use crate::thread::AgentThread;
use gpui_kit::{Context, Task, WeakEntity};
use nocterm_ai::model::content::Content;
use nocterm_ai::{images::PromptImage, session::Ticket};
use std::sync::Arc;
pub(super) struct QueueImagePreparation {
    binding: ComposerBinding,
    thread: WeakEntity<AgentThread>,
    ticket: Ticket,
    prompt_id: u64,
    saved: Arc<nocterm_ai::history::SavedPrompt>,
    images: crate::ImageDeps,
}
impl QueueImagePreparation {
    pub(super) fn new(
        binding: ComposerBinding,
        thread: WeakEntity<AgentThread>,
        ticket: Ticket,
        prompt_id: u64,
        saved: Arc<nocterm_ai::history::SavedPrompt>,
        images: crate::ImageDeps,
    ) -> Self {
        Self {
            binding,
            thread,
            ticket,
            prompt_id,
            saved,
            images,
        }
    }
    pub(super) fn launch(self, cx: &mut Context<AgentPanel>) -> Task<()> {
        let Self {
            binding,
            thread,
            ticket,
            prompt_id,
            saved,
            images,
        } = self;
        let future = cx.background_executor().spawn(async move {
            let _permit = images.work().await;
            let inspector = images.inspector();
            let mut budget = nocterm_ai::images::ImageCollectionBudget::remaining(&[])?;
            budget.check_count(
                saved
                    .images
                    .iter()
                    .filter(|content| matches!(content, Content::Image(_)))
                    .count(),
            )?;
            saved
                .images
                .iter()
                .filter_map(|block| match block {
                    Content::Image(image) => Some(
                        super::queue::decode_saved_image(&image.data).and_then(|bytes| {
                            budget.admit(bytes.len())?;
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
