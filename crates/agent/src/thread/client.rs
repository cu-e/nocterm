//! A thread is a client of the runtime: it applies the runtime's events to itself.
use super::AgentThread;
use crate::runtime::{Client, ClientState, SessionClient, SessionEvent};
use gpui_kit::{App, Context, Entity, EntityId, WeakEntity};
use std::{rc::Rc, sync::Arc};

/// `thread` as the runtime sees it.
pub(crate) fn client(thread: &Entity<AgentThread>) -> Client {
    Rc::new(thread.downgrade())
}

impl SessionClient for WeakEntity<AgentThread> {
    fn id(&self) -> EntityId {
        self.entity_id()
    }
    fn alive(&self) -> bool {
        self.upgrade().is_some()
    }
    fn emit(&self, event: SessionEvent, cx: &mut App) {
        if let Some(thread) = self.upgrade() {
            thread.update(cx, |thread, cx| thread.runtime_event(event, cx));
        }
    }
    fn state(&self, cx: &App) -> Option<ClientState> {
        let thread = self.upgrade()?;
        let thread = thread.read(cx);
        Some(ClientState {
            chat_id: thread.chat_id.clone(),
            agent_id: thread.agent_id.clone(),
            session: thread.session().clone(),
            busy: thread.session_busy(),
            leased: thread.lease.is_some(),
            activation_pending: thread.activation_pending,
            closing: thread.closing_session,
        })
    }
    fn snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        self.upgrade()?.read(cx).flush_snapshot(cx)
    }
    fn capture(&self, cx: &mut App) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)> {
        self.upgrade()?.update(cx, |thread, cx| {
            let chat = thread.flush_snapshot(cx)?;
            thread.document_revision += 1;
            Some((chat, thread.document_revision))
        })
    }
}

impl AgentThread {
    fn runtime_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Leased(lease) => self.begin_lease(lease),
            SessionEvent::Connected {
                commands,
                info,
                workdir,
            } => self.create_session(commands, *info, workdir, cx),
            SessionEvent::Update(notification) => self.session_update(&notification, cx),
            SessionEvent::Permission { request, respond } => {
                self.permission(*request, respond, cx);
            }
            SessionEvent::Tool(call) => self.handle_tool(call, cx),
            SessionEvent::Stopped(message) => self.connection_stopped(&message, cx),
            SessionEvent::Failed(message) => self.fail(&message, cx),
            SessionEvent::AgentRemoved => {
                self.activation_pending = false;
                self.fail("The agent is no longer configured.", cx);
            }
            SessionEvent::Idle => {
                self.detach_session(cx);
                self.status = "Agent idle; conversation retained.".into();
                cx.notify();
            }
            SessionEvent::CleanupUnconfirmed(error) => {
                self.status = format!("Agent process cleanup could not be confirmed: {error}");
                self.status_error = true;
                self.queue_paused = true;
                cx.notify();
            }
            SessionEvent::SessionClosed => {
                self.closing_session = false;
                cx.notify();
            }
            SessionEvent::Saved { revision } => {
                self.persisted_revision = self.persisted_revision.max(revision);
                self.persistence_error = None;
                self.dispatch_next(cx);
                cx.notify();
            }
            SessionEvent::SaveFailed(error) => {
                self.status = format!(
                    "Could not save chat: {error}. Queued messages are retained; check disk space before continuing."
                );
                self.persistence_error = Some(error);
                self.queue_paused = true;
                self.status_error = true;
                cx.notify();
            }
            SessionEvent::PolicyChanged => self.apply_permission_policy(cx),
            SessionEvent::Shutdown => self.release_resources(cx),
        }
    }
}
