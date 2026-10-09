//! Composer editing owns future context separately from active grants and saved defaults.
use super::{AgentThread, Attachment};
use gpui_kit::Context;
impl AgentThread {
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
