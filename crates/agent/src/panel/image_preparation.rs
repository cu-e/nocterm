//! Finite result work owns its injected inspector and retains original completion authority.
use super::{AgentPanel, composer_view::ComposerBinding, images::ImageAttachError};
use crate::thread::AgentThread;
use futures::future::BoxFuture;
use gpui_kit::{Context, WeakEntity};
use nocterm_ai::{images::PromptImage, session::Ticket};

pub(super) struct ImagePreparation {
    binding: ComposerBinding,
    thread: WeakEntity<AgentThread>,
    ticket: Ticket,
    work: BoxFuture<'static, Result<Vec<PromptImage>, ImageAttachError>>,
}
impl ImagePreparation {
    pub(super) fn new(
        binding: ComposerBinding,
        thread: WeakEntity<AgentThread>,
        ticket: Ticket,
        work: BoxFuture<'static, Result<Vec<PromptImage>, ImageAttachError>>,
    ) -> Self {
        Self {
            binding,
            thread,
            ticket,
            work,
        }
    }
    pub(super) fn launch(self, cx: &mut Context<AgentPanel>) {
        let Self {
            binding,
            thread,
            ticket,
            work,
        } = self;
        let future = cx.background_executor().spawn(work);
        cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_composer_images_prepared(binding, thread, ticket, result, cx)
            });
        })
        .detach();
    }
}
