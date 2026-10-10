//! Live ACP ownership is disposable; terminals and durable conversation are not.
use super::AgentThread;
use crate::runtime::{Runtime, SessionLease};
use gpui_kit::Context;
use nocterm_ai::{AgentCommands, AiSettings, BridgeRegistration, acp, session::SessionPhase};
use nocterm_ui::{ActiveAi as _, SettingsExt as _};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) struct OperationGuard(Arc<AtomicUsize>);
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl AgentThread {
    pub(crate) fn session(&self) -> &Option<acp::SessionId> {
        self.lease.as_ref().map_or(&None, |lease| &lease.session)
    }
    pub(crate) fn commands(&self) -> &Option<Arc<dyn AgentCommands>> {
        self.lease.as_ref().map_or(&None, |lease| &lease.commands)
    }
    pub(crate) fn registration(&self) -> &Option<BridgeRegistration> {
        self.lease
            .as_ref()
            .map_or(&None, |lease| &lease.registration)
    }
    pub(crate) fn session_workdir(&self) -> &Option<PathBuf> {
        self.lease.as_ref().map_or(&None, |lease| &lease.workdir)
    }
    pub(crate) fn connection_key(&self) -> Option<u64> {
        self.lease.as_ref().map(|lease| lease.connection_key)
    }
    pub(crate) fn hold_operation(&self) -> OperationGuard {
        self.operation_count.fetch_add(1, Ordering::AcqRel);
        OperationGuard(self.operation_count.clone())
    }
    pub(crate) fn session_busy(&self) -> bool {
        self.lifecycle.busy()
            || !self.permissions.is_empty()
            || !self.tools.is_empty()
            || self.operation_count.load(Ordering::Acquire) != 0
            || self.executions.active().next().is_some()
            || self.live_commands.values().any(|lease| lease.is_active())
            || self.composer.dispatchable()
    }
    /// Asks the runtime for a connection. A failed chat recovers this way.
    pub(crate) fn request_activation(&mut self, cx: &mut Context<Self>) {
        if self.archive.is_some() || self.lease.is_some() || !self.lifecycle.queue() {
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
        self.shown = shown;
        if shown {
            self.warm(cx);
        } else if !self.composer.dispatchable() && self.lifecycle.withdraw() {
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
        if self.shown
            && self.archive.is_none()
            && self.persistence_error.is_none()
            && cx.ai_enabled()
            && cx.setting::<AiSettings>().sessions.warm_start
            && self.lifecycle.phase() == SessionPhase::Detached
        {
            self.request_activation(cx);
        }
    }
    /// Detaches ACP without touching background terminals, execution jobs or shell leases.
    pub(crate) fn detach_session(&mut self, cx: &mut Context<Self>) {
        self.restore = self.restore_descriptor();
        self.finish_pending_tools();
        self.save(cx);
        // Dropping the lease releases the connection.
        if self.lease.take().is_none() {
            return;
        }
        self.lifecycle.detach();
        self.pending_controls.clear();
        self.grants.clear();
    }
    pub(crate) fn release_resources(&mut self, cx: &mut Context<Self>) {
        self.composer.queue_paused = true;
        self.cancel_pending();
        self.finalize_tool_displays();
        self.detach_session(cx);
        self.lifecycle.release();
        self.status = "Agent resources released. Conversation retained.".into();
        cx.notify();
    }
    pub(crate) fn begin_lease(&mut self, lease: SessionLease) {
        self.lease = Some(lease);
        self.lifecycle.leased();
    }
}
