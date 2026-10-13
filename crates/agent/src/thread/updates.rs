//! What the chat's connection reports: session notifications and its end.

use super::AgentThread;
use gpui_kit::Context;
use nocterm_ai::{
    acp,
    thread::{Entry, ThreadChange},
};
use nocterm_ui::ActiveAi as _;

impl AgentThread {
    /// Applies one notification from the chat's connection. Notifications
    /// for other sessions sharing the connection are ignored.
    pub(crate) fn session_update(
        &mut self,
        notification: &acp::SessionNotification,
        cx: &mut Context<Self>,
    ) {
        if self.session.opening() && self.session().is_none() {
            self.config
                .keep_control(self.session.ticket(), notification);
        }
        if self.session().as_ref() != Some(&notification.session_id)
            || !self.session.accepts_updates()
            || matches!(notification.update, acp::SessionUpdate::UserMessageChunk(_))
        {
            return;
        }
        let index = match &notification.update {
            acp::SessionUpdate::ToolCallUpdate(update) => self.state.entries().iter().position(
                |entry| matches!(entry, Entry::Tool(call) if call.tool_call_id == update.tool_call_id),
            ),
            _ => self.state.entries().len().checked_sub(1),
        };
        let configuration = matches!(
            notification.update,
            acp::SessionUpdate::ConfigOptionUpdate(_) | acp::SessionUpdate::CurrentModeUpdate(_)
        );
        match self.apply_presented_update(notification.update.clone()) {
            ThreadChange::Transcript => {
                if let Some(index) = index {
                    self.mark_dirty(index);
                }
                if let Some(index) = self.state.entries().len().checked_sub(1) {
                    self.mark_dirty(index);
                }
            }
            ThreadChange::Metadata => self.persist(cx),
            // The agent changed its configuration itself, e.g. left plan mode.
            ThreadChange::Controls if configuration => self.sync_config(true, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The chat's connection stopped. With AI turned off, the chat also
    /// forgets its transcript and what was attached.
    pub(crate) fn connection_stopped(&mut self, message: &str, cx: &mut Context<Self>) {
        self.fail(message, cx);
        self.clear_settled_disabled_document(cx);
    }
    pub(super) fn clear_settled_disabled_document(&mut self, cx: &gpui_kit::App) {
        if !cx.ai_enabled() && self.state.release_disabled() {
            self.config.disabled();
            self.composer.reset_after_disable();
        }
    }
}
