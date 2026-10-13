//! Composer editing owns future context separately from active grants and saved defaults.
use super::{AgentThread, Attachment};
use gpui_kit::Context;
use nocterm_workspace::ConnectionSummary;

impl Attachment {
    /// Whether this attachment gives the agent the saved server `summary`:
    /// the server itself, or the folder it is filed under.
    pub(crate) fn covers_server(&self, summary: &ConnectionSummary) -> bool {
        match self {
            Self::Connection(id) => summary.id.as_ref() == id,
            Self::Group(group) => summary
                .group
                .as_ref()
                .is_some_and(|name| name.as_ref() == group),
            Self::Terminal(_) | Self::UnavailableLocal(_) => false,
        }
    }
}

impl AgentThread {
    pub(crate) fn add_composer_attachment(
        &mut self,
        attachment: Attachment,
        cx: &mut Context<Self>,
    ) {
        self.composer.add_attachment_if_absent(attachment);
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn replace_composer_context(
        &mut self,
        values: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
        let old = self.composer.attachments().to_vec();
        for value in old {
            if !values.contains(&value) {
                self.attach(value, cx);
            }
        }
        for value in values {
            if !self.composer.attachments().contains(&value) {
                self.attach(value, cx);
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn clear_composer_context(&mut self, cx: &mut Context<Self>) {
        self.replace_composer_context(Vec::new(), cx);
    }
    pub(crate) fn accept_composer_images(
        &mut self,
        images: Vec<nocterm_ai::images::PromptImage>,
        cx: &mut Context<Self>,
    ) -> Result<(), nocterm_ai::images::ImageCollectionError> {
        self.composer.replace_images(images)?;
        cx.notify();
        Ok(())
    }
    pub(crate) fn remove_composer_image(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.composer.remove_image(index) {
            cx.notify();
        }
    }
    pub(crate) fn begin_composer_edit(
        &mut self,
        images: Vec<nocterm_ai::images::PromptImage>,
        attachments: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
        self.composer.begin_edit(images, attachments);
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn pause_queue(&mut self, cx: &mut Context<Self>) {
        self.composer.pause();
        cx.notify();
    }
    pub(crate) fn attachment_scope(&self) -> &[Attachment] {
        self.prompt_attachments
            .as_deref()
            .unwrap_or_else(|| self.composer.default_attachments())
    }
    pub(crate) fn end_composer_edit(&mut self, resume: bool, cx: &mut Context<Self>) {
        self.composer.end_edit();
        if resume {
            self.dispatch_next(cx);
        }
        cx.notify();
    }
}
