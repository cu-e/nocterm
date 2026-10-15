//! Image preparation runs outside the UI and belongs to one composer revision.
use super::AgentPanel;
use gpui_kit::{Context, PathPromptOptions};
use nocterm_ai::images::{ImageCollectionError, MAX_IMAGE_BYTES, PromptImage, PromptImageError};
use std::{io::Read as _, path::PathBuf};

/// Why attaching images to the composer failed.
#[derive(Debug, thiserror::Error)]
pub(super) enum ImageAttachError {
    #[error(transparent)]
    Read(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] base64::DecodeError),
    #[error(transparent)]
    Invalid(#[from] PromptImageError),
    #[error(transparent)]
    Collection(#[from] ImageCollectionError),
}

impl AgentPanel {
    pub(super) fn prepare_images(
        &mut self,
        prepare: impl std::future::Future<Output = Result<Vec<PromptImage>, ImageAttachError>>
        + Send
        + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.current() else {
            return;
        };
        if self.preparing_queue_edit()
            || !thread
                .read(cx)
                .agent_info()
                .is_some_and(|info| info.capabilities.prompt_capabilities.image)
        {
            return;
        }
        let binding = self.composer.prepare_images(thread.entity_id());
        cx.notify();
        let ticket = thread.read(cx).session_ticket();
        let owner = thread.downgrade();
        let future = cx.background_executor().spawn(prepare);
        cx.spawn(async move |panel, cx| {
            let result = future.await;
            let _ = panel.update(cx, |panel, cx| {
                panel.on_composer_images_prepared(binding, owner, ticket, result, cx)
            });
        })
        .detach();
    }
    fn on_composer_images_prepared(
        &mut self,
        binding: super::composer_view::ComposerBinding,
        owner: gpui_kit::WeakEntity<crate::thread::AgentThread>,
        ticket: nocterm_ai::session::Ticket,
        result: Result<Vec<PromptImage>, ImageAttachError>,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = owner.upgrade() else {
            return;
        };
        if !self.settings.setting::<nocterm_ai::AiSettings>(cx).enabled
            || self
                .current()
                .is_none_or(|current| current.entity_id() != thread.entity_id())
            || !self.composer.images_completed(&binding)
        {
            return;
        }
        thread.update(cx, |thread, cx| {
            if !thread.session_request_current(ticket) {
                return;
            }
            match result {
                Ok(images) => {
                    let mut collection = thread.composer().images().to_vec();
                    collection.extend(images);
                    if let Err(error) = thread.accept_composer_images(collection, cx) {
                        self.error = Some(error.to_string());
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn add_image_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let inspector = self.messages.inspector();
        self.prepare_images(
            async move { Ok(vec![PromptImage::validate(bytes, inspector.as_ref())?]) },
            cx,
        );
    }
    pub(super) fn add_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let inspector = self.messages.inspector();
        self.prepare_images(
            async move {
                paths
                    .into_iter()
                    .map(|path| {
                        let mut file = std::fs::File::open(path)?;
                        let mut bytes = Vec::new();
                        std::io::Read::take(&mut file, (MAX_IMAGE_BYTES + 1) as u64)
                            .read_to_end(&mut bytes)?;
                        Ok(PromptImage::validate(bytes, inspector.as_ref())?)
                    })
                    .collect()
            },
            cx,
        );
    }
    fn on_image_paths_picked(
        &mut self,
        owner: Option<gpui_kit::EntityId>,
        revision: u64,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.composer.revision() == revision
            && self.current().map(|thread| thread.entity_id()) == owner
        {
            self.add_images(paths, cx);
        }
    }
    pub(super) fn pick_images(&mut self, cx: &mut Context<Self>) {
        let owner = self.current().map(|thread| thread.entity_id());
        let revision = self.composer.revision();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    this.on_image_paths_picked(owner, revision, paths, cx)
                });
            }
        })
        .detach();
    }
}
