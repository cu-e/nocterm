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

impl AgentThread {
    pub(crate) fn has_pending_approvals(&self) -> bool {
        self.tools.approval_pending()
    }
    pub(crate) fn approval_notice_generation(&self) -> u64 {
        self.tools.approval_generation()
    }
    pub(crate) fn permission_cards(&self) -> Vec<PermissionCard> {
        self.tools.approval_permission_cards()
    }
    pub(crate) fn tool_cards(&self) -> Vec<ToolApprovalCard> {
        self.tools.approval_tool_cards()
    }
    fn permission_gate(&self, cx: &App) -> PermissionGate {
        PermissionGate::new(
            cx.ai_enabled(),
            self.session.accepts_updates(),
            self.session.stopped(),
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
            .tools
            .approval_receive_provider(request, respond, &gate, policy)
        {
            cx.notify();
        }
    }
    pub(crate) fn apply_permission_policy(&mut self, cx: &mut Context<Self>) {
        let gate = self.permission_gate(cx);
        let policy = cx.setting::<AiSettings>().approval.agent_permissions;
        if self.tools.approval_apply_provider_policy(&gate, policy) {
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
        if self.tools.approval_choose_provider(token, choice, &gate) {
            cx.notify();
        }
    }
    /// Whether the chat still holds connection `key`.
    fn holds(&self, key: u64) -> bool {
        self.connection_key() == Some(key)
    }
    pub(crate) fn document_admitted(&mut self, revision: u64, cx: &mut Context<Self>) {
        self.state.acknowledge_admission(revision);
        self.clear_settled_disabled_document(cx);
        cx.notify();
    }
    pub(super) fn document_save_failed(
        &mut self,
        revision: u64,
        error: crate::runtime::AgentPersistenceError,
        cx: &mut Context<Self>,
    ) {
        let message = format!(
            "Could not save chat: {error}. Queued messages are retained; check disk space before continuing."
        );
        if !self.state.failed(revision, error) {
            return;
        }
        self.status = message;
        self.composer.pause();
        self.status_error = true;
        cx.notify();
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
                if self.holds(connection) && self.session.opening() {
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
                self.session.connection_closed();
                cx.notify();
            }
            SessionEvent::SaveAdmitted { revision } => self.document_admitted(revision, cx),
            SessionEvent::Saved { revision } => {
                if self.state.published(revision) {
                    if cx.ai_enabled() {
                        self.dispatch_next(cx);
                    } else {
                        self.clear_settled_disabled_document(cx);
                    }
                }
                cx.notify();
            }
            SessionEvent::SaveFailed { revision, error } => {
                self.document_save_failed(revision, error, cx)
            }
            SessionEvent::PolicyChanged => self.apply_permission_policy(cx),
            SessionEvent::Shutdown => self.release_resources(cx),
        }
    }
}
