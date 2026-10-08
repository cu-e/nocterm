//! Live ACP ownership is disposable; terminals and durable conversation are not.
use super::AgentThread;
use crate::runtime::{Runtime, SessionLease};
use gpui_kit::Context;
use nocterm_ai::{AgentCommands, BridgeRegistration, acp};
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
        self.connecting_session
            || self.generating
            || self.auth_required
            || self.authenticating
            || !self.permissions.is_empty()
            || !self.tools.is_empty()
            || self.operation_count.load(Ordering::Acquire) != 0
            || self.executions.active().next().is_some()
            || self.live_commands.values().any(|lease| lease.is_active())
            || (!self.queue.is_empty() && !self.queue_paused && !self.queue_editing)
    }
    pub(crate) fn request_activation(&mut self, cx: &mut Context<Self>) {
        if self.activation_pending || self.lease.is_some() || self.ended() {
            return;
        }
        self.activation_pending = true;
        self.status = "Waiting for an agent slot…".into();
        let owner = cx.entity().downgrade();
        cx.defer(move |cx| {
            Runtime::global(cx).update(cx, |runtime, cx| runtime.request_activation(owner, cx));
        });
        cx.notify();
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
        self.closing_session = true;
        self.epoch += 1;
        self.dormant = true;
        self.connecting_session = false;
        self.activation_pending = false;
        self.pending_controls.clear();
        self.grants.clear();
    }
    pub(crate) fn release_resources(&mut self, cx: &mut Context<Self>) {
        self.queue_paused = true;
        self.cancel_pending();
        self.generating = false;
        self.auth_required = false;
        self.authenticating = false;
        self.detach_session(cx);
        self.accept_updates = true;
        self.stopped = false;
        self.status = "Agent resources released. Conversation retained.".into();
        cx.notify();
    }
    pub(crate) fn begin_lease(&mut self, lease: SessionLease) {
        self.lease = Some(lease);
        self.dormant = false;
        self.activation_pending = false;
        self.connecting_session = true;
    }
}
