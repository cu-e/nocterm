//! What the chat's connection reports: session notifications and its end.

use super::AgentThread;
use gpui_kit::Context;
use nocterm_ai::{
    acp,
    thread::{Entry, ThreadChange},
};
use nocterm_ui::ActiveAi as _;

/// Controls kept per session while it is being created.
const MAX_PENDING_CONTROLS: usize = 64;

impl AgentThread {
    /// Applies one notification from the chat's connection. Notifications
    /// for other sessions sharing the connection are ignored.
    pub(crate) fn session_update(
        &mut self,
        notification: &acp::SessionNotification,
        cx: &mut Context<Self>,
    ) {
        if self.connecting_session && self.session().is_none() {
            self.keep_pending_control(notification);
        }
        if self.session().as_ref() != Some(&notification.session_id)
            || !self.accept_updates
            || matches!(notification.update, acp::SessionUpdate::UserMessageChunk(_))
        {
            return;
        }
        let index = match &notification.update {
            acp::SessionUpdate::ToolCallUpdate(update) => self.state.entries.iter().position(
                |entry| matches!(entry, Entry::Tool(call) if call.tool_call_id == update.tool_call_id),
            ),
            _ => self.state.entries.len().checked_sub(1),
        };
        match self.apply_presented_update(notification.update.clone()) {
            ThreadChange::Transcript => {
                if let Some(index) = index {
                    self.mark_dirty(index);
                }
                if let Some(index) = self.state.entries.len().checked_sub(1) {
                    self.mark_dirty(index);
                }
            }
            ThreadChange::Metadata => self.persist(cx),
            _ => {}
        }
        cx.notify();
    }

    /// Keeps the latest control of each kind for a session still being
    /// created, without replaying transcript chunks.
    fn keep_pending_control(&mut self, notification: &acp::SessionNotification) {
        if !matches!(
            notification.update,
            acp::SessionUpdate::AvailableCommandsUpdate(_)
                | acp::SessionUpdate::CurrentModeUpdate(_)
                | acp::SessionUpdate::ConfigOptionUpdate(_)
        ) {
            return;
        }
        let kind = std::mem::discriminant(&notification.update);
        self.pending_controls.retain(|(id, update)| {
            id != &notification.session_id || std::mem::discriminant(update) != kind
        });
        if self.pending_controls.len() == MAX_PENDING_CONTROLS {
            self.pending_controls.remove(0);
        }
        self.pending_controls
            .push((notification.session_id.clone(), notification.update.clone()));
    }

    /// The chat's connection stopped. With AI turned off, the chat also
    /// forgets its transcript and what was attached.
    pub(crate) fn connection_stopped(&mut self, message: &str, cx: &mut Context<Self>) {
        self.fail(message, cx);
        if !cx.ai_enabled() {
            self.state = Default::default();
            self.attachments.clear();
            self.images.clear();
        }
    }
}
