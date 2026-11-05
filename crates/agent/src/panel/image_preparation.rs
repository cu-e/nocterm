//! Image admission and finite preparation retain their original composer and producer.
use super::{AgentPanel, composer_view::ComposerBinding, images::ImageAttachError};
use crate::thread::AgentThread;
use futures::future::BoxFuture;
use gpui_kit::{App, Context, EntityId, WeakEntity};
use nocterm_agent_chat::conversation::{ConversationComposer, ConversationSource};
use nocterm_ai::{images::PromptImage, session::Ticket};
use std::rc::Rc;

pub(super) enum ImageTarget {
    Local {
        thread: WeakEntity<AgentThread>,
        ticket: Ticket,
    },
    Conversation {
        source: Rc<dyn ConversationSource>,
        composer: Rc<dyn ConversationComposer>,
    },
}
impl ImageTarget {
    pub(super) fn capture(
        source: Option<Rc<dyn ConversationSource>>,
        local: Option<gpui_kit::Entity<AgentThread>>,
        cx: &App,
    ) -> Option<Self> {
        if let Some(source) = source {
            let mut accepts = false;
            source
                .with_view(cx, &mut |view| {
                    accepts = view.can_prompt && view.composer.attach
                })
                .ok()?;
            if !accepts {
                return None;
            }
            let composer = source.composer(cx)?;
            composer.revision(cx).ok()?;
            return Some(Self::Conversation { source, composer });
        }
        let thread = local?;
        thread
            .read(cx)
            .agent_info()
            .filter(|info| info.capabilities.image_prompts)?;
        Some(Self::Local {
            ticket: thread.read(cx).session_ticket(),
            thread: thread.downgrade(),
        })
    }
    pub(super) fn identity(&self) -> EntityId {
        match self {
            Self::Local { thread, .. } => thread.entity_id(),
            Self::Conversation { source, .. } => source.identity(),
        }
    }
    pub(super) fn images(&self, cx: &App) -> Vec<PromptImage> {
        match self {
            Self::Local { thread, .. } => thread
                .upgrade()
                .map(|t| t.read(cx).composer().images().to_vec())
                .unwrap_or_default(),
            Self::Conversation { composer, .. } => composer.images(cx),
        }
    }
    pub(super) fn accept(
        self,
        images: Vec<PromptImage>,
        cx: &mut App,
    ) -> Result<(), ImageAttachError> {
        match self {
            Self::Local { thread, ticket } => {
                if let Some(thread) = thread.upgrade() {
                    thread.update(cx, |thread, cx| {
                        if thread.session_request_current(ticket) {
                            let mut collection = thread.composer().images().to_vec();
                            collection.extend(images);
                            thread.accept_composer_images(collection, cx)?;
                        }
                        Ok(())
                    })
                } else {
                    Ok(())
                }
            }
            Self::Conversation { source, composer } => {
                source.with_view(cx, &mut |_| {})?;
                // Append to the current collection. Choosing a model or removing another
                // image does not cancel this acquisition; the source rechecks its budget.
                composer.accept_images(images, cx)?;
                Ok(())
            }
        }
    }
}

pub(super) struct ImagePreparation {
    binding: ComposerBinding,
    target: ImageTarget,
    work: BoxFuture<'static, Result<Vec<PromptImage>, ImageAttachError>>,
    permit: crate::image_deps::ImageWorkPermit,
}
impl ImagePreparation {
    pub(super) fn new(
        binding: ComposerBinding,
        target: ImageTarget,
        work: BoxFuture<'static, Result<Vec<PromptImage>, ImageAttachError>>,
        permit: crate::image_deps::ImageWorkPermit,
    ) -> Self {
        Self {
            binding,
            target,
            work,
            permit,
        }
    }
    pub(super) fn launch(self, cx: &mut Context<AgentPanel>) {
        let Self {
            binding,
            target,
            work,
            permit,
        } = self;
        let future = cx.background_executor().spawn(async move {
            let _permit = permit;
            work.await
        });
        cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_composer_images_prepared(binding, target, result, cx)
            });
        })
        .detach();
    }
}
