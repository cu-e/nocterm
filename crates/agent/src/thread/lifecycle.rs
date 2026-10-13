//! Live ACP ownership is disposable; terminals and durable conversation are not.
use super::AgentThread;
use crate::runtime::{Runtime, SessionLease};
use gpui_kit::Context;
use nocterm_ai::{AgentCommands, AiSettings, BridgeRegistration, acp, session::SessionPhase};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use std::{path::PathBuf, sync::Arc};

pub(crate) use super::session_driver::OperationGuard;
impl AgentThread {
    pub(crate) fn session(&self) -> &Option<acp::SessionId> {
        self.session.session()
    }
    pub(crate) fn commands(&self) -> &Option<Arc<dyn AgentCommands>> {
        self.session.commands()
    }
    pub(crate) fn registration(&self) -> &Option<BridgeRegistration> {
        self.session.registration()
    }
    pub(crate) fn session_workdir(&self) -> &Option<PathBuf> {
        self.session.workdir()
    }
    pub(crate) fn connection_key(&self) -> Option<u64> {
        self.session.connection_key()
    }
    pub(crate) fn hold_operation(&self) -> OperationGuard {
        self.session.hold_operation()
    }
    pub(crate) fn session_busy(&self) -> bool {
        self.session.busy() || self.tools.busy() || self.composer.dispatchable()
    }
    /// Asks the runtime for a connection. A failed chat recovers this way.
    pub(crate) fn request_activation(&mut self, cx: &mut Context<Self>) {
        if self.is_archived() || self.session.leased() || !self.session.queue_activation() {
            return;
        }
        self.status_error = false;
        self.status = "Waiting for an agent slot…".into();
        let owner = super::client(&cx.entity());
        cx.defer(move |cx| {
            Runtime::global(cx).update(cx, |runtime, cx| runtime.request_activation(owner, cx));
        });
        cx.notify();
    }
    /// A panel starts or stops showing the chat. A chat hidden before it
    /// was admitted gives up its place unless it has messages to send.
    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.session.set_shown(shown);
        if shown {
            self.warm(cx);
        } else if !self.composer.dispatchable() && self.session.withdraw_if_idle() {
            self.status.clear();
            cx.notify();
        }
    }
    /// With warm start on, a shown chat connects at once, so its model and
    /// options can be chosen before the first message. A chat opened from
    /// the history connects too once it is loaded; its restored queue stays
    /// paused until the user sends. A failed chat waits for the next
    /// message, so a broken agent is not restarted on every visit, and a
    /// chat that cannot save waits until it can.
    pub(crate) fn warm(&mut self, cx: &mut Context<Self>) {
        if self.session.shown()
            && !self.is_archived()
            && self.state.persistence_error().is_none()
            && cx.ai_enabled()
            && cx.setting::<AiSettings>().sessions.warm_start
            && self.session.phase() == SessionPhase::Detached
        {
            self.request_activation(cx);
        }
    }
    /// Detaches ACP without touching background terminals, execution jobs or shell leases.
    pub(crate) fn detach_session(&mut self, cx: &mut Context<Self>) {
        self.session.import_restore(self.restore_descriptor());
        self.finish_pending_tools();
        self.save(cx);
        // Dropping the lease releases the connection.
        if !self.session.release_lease() {
            return;
        }
        self.session.detach();
        self.config.detached();
        for call in self.tools.approval_end_session() {
            self.finish(call, Err("Request cancelled.".into()));
        }
    }
    pub(crate) fn release_resources(&mut self, cx: &mut Context<Self>) {
        self.composer.pause();
        self.cancel_pending();
        self.finalize_tool_displays();
        self.detach_session(cx);
        self.session.release();
        self.status = "Agent resources released. Conversation retained.".into();
        cx.notify();
    }
    pub(crate) fn begin_lease(&mut self, lease: SessionLease) {
        self.session.accept_lease(lease);
    }
}
