//! The runtime port adapter keeps event delivery synchronous and ordered.
use super::AgentThread;
use crate::runtime::{Client, ClientState, SessionClient, SessionEvent};
use gpui_kit::{App, Entity, EntityId, WeakEntity};
use std::{rc::Rc, sync::Arc};
/// `thread` as the runtime sees it.
pub(crate) fn client(thread: &Entity<AgentThread>) -> Client {
    Rc::new(ThreadClient(thread.downgrade()))
}

struct ThreadClient(WeakEntity<AgentThread>);

impl SessionClient for ThreadClient {
    fn id(&self) -> EntityId {
        self.0.entity_id()
    }
    fn alive(&self) -> bool {
        self.0.upgrade().is_some()
    }
    fn emit(&self, event: SessionEvent, cx: &mut App) {
        if let Some(thread) = self.0.upgrade() {
            thread.update(cx, |thread, cx| thread.runtime_event(event, cx));
        }
    }
    fn state(&self, cx: &App) -> Option<ClientState> {
        let thread = self.0.upgrade()?;
        let thread = thread.read(cx);
        Some(ClientState {
            chat_id: thread.state.chat_id().to_owned(),
            workdir: thread
                .session
                .restore()
                .as_ref()
                .filter(|restore| {
                    !thread.state.entries().is_empty() && restore.workdir.is_absolute()
                })
                .map(|restore| restore.workdir.clone()),
            agent_id: thread.state.agent_id().to_owned(),
            session: thread.session().clone(),
            busy: thread.session_busy(),
            leased: thread.session.leased(),
            activation_pending: thread.session.queued(),
            closing: thread.session.closing(),
            shown: thread.session.shown(),
        })
    }
    fn snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        self.0.upgrade()?.read(cx).flush_snapshot(cx)
    }
    fn capture(&self, cx: &mut App) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)> {
        self.0
            .upgrade()?
            .update(cx, |thread, cx| thread.capture_pending(cx))
    }
}
