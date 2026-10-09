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
