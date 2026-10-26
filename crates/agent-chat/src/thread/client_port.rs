//! The runtime port adapter keeps event delivery synchronous and ordered.
use super::AgentThread;
use crate::runtime::{Client, ClientState, SessionClient, SessionEvent};
use gpui::{App, Entity, EntityId, WeakEntity};
use std::{rc::Rc, sync::Arc};
/// `thread` as the runtime sees it.
pub fn client(thread: &Entity<AgentThread>) -> Client {
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
        let resume = thread
            .session
            .restore()
            .as_ref()
            .filter(|restore| !thread.state.entries().is_empty() && restore.workdir.is_absolute())
            .map(|restore| restore.workdir.clone());
        let origin =
            nocterm_ai::execution_origin::ExecutionOrigin::new(thread.state.key().clone(), resume);
        Some(ClientState::new(
            origin,
            thread.state.agent_id().to_owned(),
            thread.session().clone(),
            thread.session_busy(),
            thread.session.leased(),
            thread.session.queued(),
            thread.session.closing(),
            thread.session.shown(),
        ))
    }
    fn history_usage(&self, cx: &App) -> Option<nocterm_agent_runtime::HistoryUsage> {
        let thread = self.0.upgrade()?;
        let thread = thread.read(cx);
        Some(nocterm_agent_runtime::HistoryUsage::new(
            thread.resident_history_bytes(),
            thread.document().updated(),
            thread.session.shown(),
        ))
    }
    fn evict_history(&self, cx: &mut App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|thread| thread.update(cx, |thread, cx| thread.evict_history(cx)))
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
