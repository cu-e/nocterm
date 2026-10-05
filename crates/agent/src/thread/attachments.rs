//! Composer editing owns future context separately from active grants and saved defaults.
use super::{AgentThread, Attachment};
use gpui_kit::Context;
pub(super) struct ComposerDefaults {
    attachments: Vec<Attachment>,
    images: Vec<nocterm_ai::images::PromptImage>,
}
impl AgentThread {
    pub(crate) fn default_attachments(&self) -> &[Attachment] {
        self.composer_defaults
            .as_ref()
            .map_or(&self.attachments, |draft| &draft.attachments)
    }
    pub(crate) fn attachment_scope(&self) -> &[Attachment] {
        self.prompt_attachments
            .as_deref()
            .unwrap_or_else(|| self.default_attachments())
    }
    pub(crate) fn begin_composer_edit(
        &mut self,
        images: Vec<nocterm_ai::images::PromptImage>,
        attachments: Vec<Attachment>,
    ) {
        debug_assert!(self.composer_defaults.is_none());
        self.composer_defaults = Some(ComposerDefaults {
            images: std::mem::replace(&mut self.images, images),
            attachments: std::mem::replace(&mut self.attachments, attachments),
        });
        self.queue_editing = true;
    }
    pub(crate) fn end_composer_edit(&mut self, resume: bool, cx: &mut Context<Self>) {
        if let Some(defaults) = self.composer_defaults.take() {
            self.images = defaults.images;
            self.attachments = defaults.attachments;
        }
        self.queue_editing = false;
        if resume {
            self.dispatch_next(cx);
        }
        cx.notify();
    }
}
