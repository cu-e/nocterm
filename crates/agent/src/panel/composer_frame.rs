//! What the composer of the open chat offers, read once per drawing from its thread.
use super::composer_view::ComposerView;
use crate::thread::AgentThread;
use gpui_kit::{App, Entity, EntityId, WeakEntity};
use nocterm_ai::model::usage::ContextUsage;
use nocterm_ai::session::Ticket;

pub(super) struct ComposerFrame {
    origin: EntityId,
    can_prompt: bool,
    can_stop: bool,
    usage: Option<ContextUsage>,
}
impl ComposerFrame {
    pub(super) fn read(thread: &Entity<AgentThread>, cx: &App) -> Self {
        let state = thread.read(cx);
        Self {
            origin: thread.entity_id(),
            can_prompt: !state.session_sign_in_required(),
            can_stop: state.session_generating() || state.queued_prompt_ids().len() != 0,
            usage: state.transcript().usage.clone(),
        }
    }
    /// The chat the composer sends to and its controls act on.
    pub(super) fn origin(&self) -> EntityId {
        self.origin
    }
    /// The context usage the usage ring shows.
    pub(super) fn usage(&self) -> Option<&ContextUsage> {
        self.usage.as_ref()
    }
    /// Whether the send button stops the run instead: nothing is typed, attached or edited
    /// and there is a run to stop.
    pub(super) fn stops(&self, composer: &ComposerView, images: bool, cx: &App) -> bool {
        self.can_stop && composer.text(cx).trim().is_empty() && !images && !composer.editing()
    }
    /// Whether sending is unavailable: the chat takes no prompts now or images are being
    /// prepared.
    pub(super) fn send_disabled(&self, stops: bool, composer: &ComposerView) -> bool {
        !stops && (!self.can_prompt || composer.preparing())
    }
}

/// The stop the send button shows: the chat and the session it was drawn for. A stop drawn
/// for an earlier session of the chat stops nothing.
#[derive(Clone)]
pub(super) struct RenderedStop {
    thread: WeakEntity<AgentThread>,
    ticket: Ticket,
}
impl RenderedStop {
    pub(super) fn capture(thread: &Entity<AgentThread>, cx: &App) -> Self {
        Self {
            thread: thread.downgrade(),
            ticket: thread.read(cx).session_ticket(),
        }
    }
    pub(super) fn activate(&self, cx: &mut App) {
        let Some(thread) = self.thread.upgrade() else {
            return;
        };
        if thread.read(cx).session_ticket() == self.ticket {
            thread.update(cx, |thread, cx| thread.stop(cx));
        }
    }
}
