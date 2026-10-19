//! The runtime port adapter keeps event delivery synchronous and ordered.
use super::AgentThread;
use crate::runtime::{Client, ClientState, SessionClient, SessionEvent};
use gpui_kit::{App, Entity, EntityId, WeakEntity};
use std::rc::Rc;
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
    fn prepare_startup(
        &self,
        request: nocterm_ai::connection::ManagedStartupRequest<'_>,
        cx: &mut App,
    ) -> Result<nocterm_ai::connection::ManagedStartup, nocterm_ai::history::source::SourceError>
    {
        use nocterm_ai::history::source::SourceError;
        let thread = self.0.upgrade().ok_or(SourceError::Unavailable)?;
        thread.update(cx, |thread, _| {
            let state = thread.state.transcript().ok_or(SourceError::Unavailable)?;
            let original = state.source().capture()?;
            let facts = original.context()?.prepare_startup(request)?;
            nocterm_ai::connection::ManagedStartup::new(facts, original)
        })
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
            chat_id: thread.state.chat_id()?.to_string(),
            workdir: thread
                .session
                .restore()
                .as_ref()
                .filter(|restore| !thread.state.rows().is_empty() && restore.workdir.is_absolute())
                .map(|restore| restore.workdir.clone()),
            agent_id: thread.state.agent_id()?.to_string(),
            session: thread.session().clone(),
            busy: thread.session_busy(),
            leased: thread.session.leased(),
            activation_pending: thread.session.queued(),
            closing: thread.session.closing(),
            shown: thread.session.shown(),
        })
    }
    fn capture_into(
        &self,
        admission: &crate::runtime::CaptureAdmission,
        cx: &mut App,
    ) -> Result<Option<u64>, crate::runtime::CaptureError> {
        let Some(thread) = self.0.upgrade() else {
            return Ok(None);
        };
        thread.update(cx, |thread, cx| thread.capture_into(admission, cx))
    }
}
