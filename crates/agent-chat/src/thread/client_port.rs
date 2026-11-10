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
    fn chat(&self) -> Option<gpui::AnyWeakEntity> {
        Some(self.0.clone().into())
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
    fn history_payload(&self, cx: &App) -> Option<u64> {
        let thread = self.0.upgrade()?;
        let thread = thread.read(cx);
        (!thread.is_archived() && thread.history_eviction_gate().is_permitted())
            .then(|| thread.payloads.order())
            .flatten()
    }
    fn evict_history(&self, cx: &mut App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|thread| thread.update(cx, |thread, cx| thread.evict_history(cx)))
    }
    fn snapshot(
        &self,
        cx: &mut App,
    ) -> Result<Option<Arc<nocterm_ai::history::SharedChat>>, nocterm_ai::storage::AgentStorageError>
    {
        let Some(thread) = self.0.upgrade() else {
            return Ok(None);
        };
        thread.update(cx, |thread, cx| thread.flush_snapshot(cx))
    }
    fn capture(
        &self,
        cx: &mut App,
    ) -> Result<
        Option<(Arc<nocterm_ai::history::SharedChat>, u64)>,
        nocterm_ai::storage::AgentStorageError,
    > {
        let Some(thread) = self.0.upgrade() else {
            return Ok(None);
        };
        thread.update(cx, |thread, cx| thread.capture_pending(cx))
    }
}
