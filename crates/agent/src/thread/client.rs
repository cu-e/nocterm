//! A thread is a client of the runtime: it applies the runtime's events to itself.
use super::{
    AgentThread, PermissionCard, ToolApprovalCard, approval_inbox::ApprovalToken,
    permissions::PermissionGate,
};
use crate::runtime::{Client, ClientState, SessionClient, SessionEvent};
use gpui_kit::{App, Context, Entity, EntityId, WeakEntity};
use nocterm_ai::{AiSettings, PermissionResponder, acp};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
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
            chat_id: thread.chat_id.clone(),
            workdir: thread
                .restore
                .as_ref()
                .filter(|restore| !thread.state.entries.is_empty() && restore.workdir.is_absolute())
                .map(|restore| restore.workdir.clone()),
            agent_id: thread.agent_id.clone(),
            session: thread.session().clone(),
            busy: thread.session_busy(),
            leased: thread.lease.is_some(),
            activation_pending: thread.lifecycle.queued(),
            closing: thread.lifecycle.closing(),
            shown: thread.shown,
        })
    }
    fn snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        self.0.upgrade()?.read(cx).flush_snapshot(cx)
    }
    fn capture(&self, cx: &mut App) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)> {
        self.0.upgrade()?.update(cx, |thread, cx| {
            let chat = thread.flush_snapshot(cx)?;
            thread.document_revision += 1;
            Some((chat, thread.document_revision))
        })
    }
}

impl AgentThread {
    pub(crate) fn has_pending_approvals(&self) -> bool {
        self.approvals.pending()
    }
    pub(crate) fn approval_notice_generation(&self) -> u64 {
        self.approvals.generation()
    }
    pub(crate) fn permission_cards(&self) -> Vec<PermissionCard> {
        self.approvals.permission_cards()
    }
    pub(crate) fn tool_cards(&self) -> Vec<ToolApprovalCard> {
        self.approvals.tool_cards()
    }
    fn permission_gate(&self, cx: &App) -> PermissionGate {
        PermissionGate::new(
            cx.ai_enabled(),
            self.lifecycle.accepts_updates(),
            self.lifecycle.stopped(),
            self.session().clone(),
        )
    }
    pub(crate) fn permission(
        &mut self,
        request: acp::RequestPermissionRequest,
        respond: PermissionResponder,
        cx: &mut Context<Self>,
    ) {
        let gate = self.permission_gate(cx);
        let policy = cx.setting::<AiSettings>().approval.agent_permissions;
        if self
            .approvals
            .receive_provider(request, respond, &gate, policy)
        {
            cx.notify();
        }
    }
    pub(crate) fn apply_permission_policy(&mut self, cx: &mut Context<Self>) {
        let gate = self.permission_gate(cx);
        let policy = cx.setting::<AiSettings>().approval.agent_permissions;
        if self.approvals.apply_provider_policy(&gate, policy) {
            cx.notify();
        }
    }
    pub(crate) fn choose_permission_request(
        &mut self,
        token: ApprovalToken,
        choice: Option<acp::PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        let gate = self.permission_gate(cx);
        if self.approvals.choose_provider(token, choice, &gate) {
            cx.notify();
        }
    }
    /// Whether the chat still holds connection `key`.
    fn holds(&self, key: u64) -> bool {
        self.connection_key() == Some(key)
    }
    fn runtime_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Leased(lease) => self.begin_lease(lease),
            SessionEvent::Connected {
                connection,
                commands,
                info,
                workdir,
            } => {
                if self.holds(connection) && self.lifecycle.opening() {
                    self.create_session(commands, *info, workdir, cx);
                }
            }
            SessionEvent::Update(notification) => self.session_update(&notification, cx),
            SessionEvent::Permission { request, respond } => {
                self.permission(*request, respond, cx);
            }
            SessionEvent::Tool(call) => self.handle_tool(call, cx),
            SessionEvent::ToolRejected(rejected) => self.record_rejected_tool(rejected, cx),
            SessionEvent::Stopped {
                connection,
                message,
            } => {
                if self.holds(connection) {
                    self.connection_stopped(&message, cx);
                }
            }
            SessionEvent::Failed(message) => self.fail(&message, cx),
            SessionEvent::AgentRemoved => {
                self.fail("The agent is no longer configured.", cx);
            }
            SessionEvent::Idle { connection } => {
                if !self.holds(connection) {
                    return;
                }
                self.detach_session(cx);
                self.status = "Agent idle; conversation retained.".into();
                cx.notify();
            }
            SessionEvent::CleanupUnconfirmed(error) => {
                self.status = format!("Agent process cleanup could not be confirmed: {error}");
                self.status_error = true;
                self.composer.pause();
                cx.notify();
            }
            SessionEvent::SessionClosed => {
                self.lifecycle.closed();
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
                self.persistence_error = Some(error.to_string());
                self.composer.pause();
                self.status_error = true;
                cx.notify();
            }
            SessionEvent::PolicyChanged => self.apply_permission_policy(cx),
            SessionEvent::Shutdown => self.release_resources(cx),
        }
    }
}
