//! Image preparation runs outside the UI and belongs to one composer revision.
use super::AgentPanel;
use gpui_kit::{Context, PathPromptOptions};
use nocterm_ai::images::{ImageCollectionError, MAX_IMAGE_BYTES, PromptImage, PromptImageError};
use nocterm_ui::ActiveAi as _;
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
                .info
                .as_ref()
                .is_some_and(|info| info.capabilities.prompt_capabilities.image)
        {
            return;
        }
        self.composer.pending_images += 1;
        cx.notify();
        let ticket = thread.read(cx).lifecycle.ticket();
        let id = thread.entity_id();
        let revision = self.composer.revision;
        let thread = thread.downgrade();
        let future = cx.background_executor().spawn(prepare);
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |panel, cx| {
                if panel.composer.revision != revision
                    || !cx.ai_enabled()
                    || panel
                        .current()
                        .is_none_or(|thread| thread.entity_id() != id)
                {
                    return;
                }
                panel.composer.pending_images = panel.composer.pending_images.saturating_sub(1);
                let _ = thread.update(cx, |thread, cx| {
                    if !thread.lifecycle.session_current(ticket) {
                        return;
                    }
                    match result {
                        Ok(images) => {
                            let mut collection = thread.composer().images().to_vec();
                            collection.extend(images);
                            match thread.accept_composer_images(collection, cx) {
                                Ok(()) => {}
                                Err(error) => panel.error = Some(error.to_string()),
                            }
                        }
                        Err(error) => panel.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn add_image_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let inspector = self.image_cache.inspector();
        self.prepare_images(
            async move { Ok(vec![PromptImage::validate(bytes, inspector.as_ref())?]) },
            cx,
        );
    }
    pub(super) fn add_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let inspector = self.image_cache.inspector();
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
    pub(super) fn pick_images(&mut self, cx: &mut Context<Self>) {
        let owner = self.current().map(|thread| thread.entity_id());
        let revision = self.composer.revision;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    if this.composer.revision == revision
                        && this.current().map(|thread| thread.entity_id()) == owner
                    {
                        this.add_images(paths, cx);
                    }
                });
            }
        })
        .detach();
    }
}
