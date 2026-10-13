//! Session-authorized terminal work retains its original scope, jobs and receipts.
use super::{
    Attachment, SignInWait, ToolApprovalCard,
    approval_inbox::{ApprovalInbox, ApprovalToken, ToolDecision, ToolGate},
    bound_call::BoundToolCall,
    permissions::{PermissionCard, PermissionGate},
};
use gpui_kit::{EntityId, Task};
use nocterm_ai::{
    ApprovalPolicy, ApprovalSettings, BridgeCall, TerminalCall, ToolFailure, acp, thread::Entry,
    tool_display::ToolDisplay,
};
use nocterm_workspace::{ExpectedSession, LiveCommandLease};
use std::sync::Arc;
mod command_jobs;
mod live_observations;
mod terminal_sessions;
mod turn_context;
use command_jobs::CommandJobs;
pub(super) use command_jobs::{CommandSnapshot, CommandStart};
use live_observations::LiveObservations;
use terminal_sessions::TerminalSessions;
use turn_context::TurnContext;
#[derive(Default)]
pub(super) struct ToolRuntime {
    approvals: ApprovalInbox,
    turn: TurnContext,
    commands: CommandJobs,
    live: LiveObservations,
    terminals: TerminalSessions,
    displays: super::tool_display::ToolDisplays,
}
impl ToolRuntime {
    pub(super) fn busy(&self) -> bool {
        self.approvals.pending() || self.commands.active().next().is_some() || self.live.active()
    }
    pub(super) fn capture_scope(&mut self, scope: Vec<Attachment>) {
        self.turn.capture_scope(scope);
    }
    pub(super) fn clear_scope(&mut self) {
        self.turn.clear_scope();
    }
    #[cfg(test)]
    pub(super) fn prompt_scope(&self) -> Option<&[Attachment]> {
        self.turn.prompt_scope()
    }
    pub(super) fn attachment_scope<'a>(&'a self, defaults: &'a [Attachment]) -> &'a [Attachment] {
        self.turn.prompt_scope().unwrap_or(defaults)
    }
    pub(super) fn detach_attachment(&mut self, attachment: &Attachment) {
        self.turn.detach_attachment(attachment);
    }
    pub(super) fn terminal_id(&mut self, key: &str) -> String {
        self.turn.terminal_id(key)
    }
    pub(super) fn server_id(&mut self, key: &str) -> String {
        self.turn.server_id(key)
    }
    pub(super) fn resolve_server(&self, key: &str) -> Option<&str> {
        self.turn.resolve_server(key)
    }
    pub(super) fn account_context(&mut self, bytes: usize) {
        self.turn.account_context(bytes);
    }
    pub(super) fn account_outcome(&mut self, bytes: usize) {
        self.turn.account_outcome(bytes);
    }
    pub(super) fn usage(&self) -> (usize, usize) {
        self.turn.usage()
    }
    pub(super) fn background(&self) -> &[EntityId] {
        self.terminals.background()
    }
    pub(super) fn remember_opened(&mut self, item: EntityId) {
        self.terminals.remember_opened(item);
    }
    pub(super) fn forget_opened(&mut self, item: EntityId) {
        self.terminals.forget_opened(item);
    }
    pub(super) fn take_close_effects(&mut self) -> Vec<EntityId> {
        self.terminals.take_close_effects()
    }
    pub(super) fn prune_background(&mut self, retained: &[EntityId]) -> Vec<EntityId> {
        self.terminals.prune_background(retained)
    }
    pub(super) fn sign_ins(&self) -> &[SignInWait] {
        self.terminals.sign_ins()
    }
    pub(super) fn observe_prompt(&mut self, item: EntityId, wait: Option<SignInWait>) -> bool {
        self.terminals.observe_prompt(item, wait)
    }
    pub(super) fn clear_wait(&mut self, expected: ExpectedSession) {
        self.terminals.clear_wait(expected);
    }
    pub(super) fn reconnecting(&self, item: &EntityId) -> bool {
        self.terminals.reconnecting(item)
    }
    pub(super) fn reconnect_receipt(&self, expected: ExpectedSession) -> Option<ExpectedSession> {
        self.terminals.reconnect_receipt(expected)
    }
    pub(super) fn accept_reconnect(&mut self, expected: ExpectedSession) {
        self.terminals.accept_reconnect(expected);
    }
    pub(super) fn finish_reconnect(&mut self, expected: ExpectedSession) {
        self.terminals.finish_reconnect(expected);
    }
    pub(super) fn install_live(&mut self, id: String, lease: Arc<LiveCommandLease>) {
        self.live.install(id, lease);
    }
    pub(super) fn finish_live_if_same(&mut self, id: &str, lease: &Arc<LiveCommandLease>) {
        self.live.finish_if_same(id, lease);
    }
    pub(super) fn live_empty(&self) -> bool {
        self.live.empty()
    }
    pub(super) fn revoke_detached_live(&mut self, attached: &[String]) {
        self.live.revoke_detached(attached);
    }
    pub(super) fn cancel_execution(&mut self) {
        self.commands.cancel_all();
        self.live.cancel_all();
    }
    pub(super) fn active_commands(
        &self,
    ) -> impl Iterator<Item = (&str, &Arc<dyn nocterm_session::HostExec>)> {
        self.commands.active()
    }
    pub(super) fn start_command(
        &mut self,
        terminal: String,
        executor: Arc<dyn nocterm_session::HostExec>,
        request: nocterm_session::ExecRequest,
        deadline: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<CommandStart, ToolFailure> {
        self.commands.start(terminal, executor, request, deadline)
    }
    pub(super) fn command_snapshot(
        &self,
        id: &str,
        terminal: &str,
    ) -> Result<CommandSnapshot, ToolFailure> {
        self.commands.snapshot(id, terminal)
    }
    pub(super) fn cancel_command(
        &self,
        id: &str,
        terminal: &str,
    ) -> Result<CommandSnapshot, ToolFailure> {
        self.commands.cancel(id, terminal)
    }
    pub(super) fn revoke_command_terminal(&self, id: &str) {
        self.commands.revoke_terminal(id);
    }
    pub(super) fn has_watch(&self) -> bool {
        self.commands.has_watch()
    }
    pub(super) fn install_watch(&mut self, watch: Task<()>) {
        self.commands.install_watch(watch);
    }
    pub(super) fn finish_watch(&mut self) {
        self.commands.finish_watch();
    }
    pub(super) fn prepare_displays(&mut self, turn: u64, rows: usize) {
        self.displays.prepare(turn, rows);
    }
    pub(super) fn normalize_displays(
        &mut self,
        entries: &[Entry],
    ) -> Vec<super::document::ToolMetadataPatch> {
        self.displays.normalize(entries)
    }
    pub(super) fn record_display(
        &mut self,
        server: String,
        request: (String, serde_json::Value),
        display: ToolDisplay,
    ) -> Option<u64> {
        self.displays.record_raw(server, request, display)
    }
    pub(super) fn finish_display(
        &mut self,
        token: Option<(u64, u64)>,
        result: Result<serde_json::Value, ToolFailure>,
    ) {
        self.displays.finish(token, result);
    }
    pub(super) fn finalize_displays(&mut self, turn: u64) {
        self.displays.finalize(turn);
    }
    pub(super) fn clear_displays(&mut self) {
        self.displays = Default::default();
    }
    pub(super) fn approval_receive_provider(
        &mut self,
        request: acp::RequestPermissionRequest,
        respond: nocterm_ai::PermissionResponder,
        gate: &PermissionGate,
        policy: ApprovalPolicy,
    ) -> bool {
        self.approvals
            .receive_provider(request, respond, gate, policy)
    }
    pub(super) fn approval_apply_provider_policy(
        &mut self,
        gate: &PermissionGate,
        policy: ApprovalPolicy,
    ) -> bool {
        self.approvals.apply_provider_policy(gate, policy)
    }
    pub(super) fn approval_choose_provider(
        &mut self,
        token: ApprovalToken,
        choice: Option<acp::PermissionOptionId>,
        gate: &PermissionGate,
    ) -> bool {
        self.approvals.choose_provider(token, choice, gate)
    }
    pub(super) fn approval_receive_tool(
        &mut self,
        call: BoundToolCall,
        gate: &ToolGate,
        policy: &ApprovalSettings,
    ) -> ToolDecision {
        self.approvals.receive_tool(call, gate, policy)
    }
    pub(super) fn approval_choose_tool(
        &mut self,
        token: ApprovalToken,
        allow: bool,
        grant: bool,
        gate: &ToolGate,
    ) -> ToolDecision {
        self.approvals.choose_tool(token, allow, grant, gate)
    }
    pub(super) fn approval_tool(&self, token: ApprovalToken) -> Option<&TerminalCall> {
        self.approvals.tool(token)
    }
    pub(super) fn approval_cancel_pending(&mut self) -> Vec<BridgeCall> {
        self.approvals.cancel_pending()
    }
    pub(super) fn approval_revoke_scope(&mut self) {
        self.approvals.revoke_scope()
    }
    pub(super) fn approval_end_session(&mut self) -> Vec<BridgeCall> {
        self.approvals.end_session()
    }
    pub(super) fn approval_pending(&self) -> bool {
        self.approvals.pending()
    }
    pub(super) fn approval_generation(&self) -> u64 {
        self.approvals.generation()
    }
    pub(super) fn approval_permission_cards(&self) -> Vec<PermissionCard> {
        self.approvals.permission_cards()
    }
    pub(super) fn approval_tool_cards(&self) -> Vec<ToolApprovalCard> {
        self.approvals.tool_cards()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn running_scope_does_not_follow_future_defaults_and_detachment_revokes_only_that_scope() {
        let mut tools = ToolRuntime::default();
        let fixed = Attachment::Group("accepted turn".into());
        let future = vec![Attachment::Group("next turn".into())];
        tools.capture_scope(vec![fixed.clone()]);
        assert_eq!(
            tools.attachment_scope(&future),
            std::slice::from_ref(&fixed)
        );
        tools.detach_attachment(&fixed);
        assert!(tools.attachment_scope(&future).is_empty());
        assert_eq!(future, vec![Attachment::Group("next turn".into())]);
        tools.clear_scope();
        assert_eq!(tools.attachment_scope(&future), future);
    }
    #[test]
    fn clearing_a_turn_preserves_opaque_terminal_identity_and_usage_accounting() {
        let mut tools = ToolRuntime::default();
        let id = tools.terminal_id("original item");
        let server = tools.server_id("profile");
        tools.account_context(40);
        tools.account_outcome(9);
        tools.capture_scope(vec![Attachment::Group("turn".into())]);
        tools.clear_scope();
        assert_eq!(tools.terminal_id("original item"), id);
        assert_eq!(tools.resolve_server(&server), Some("profile"));
        assert_ne!(tools.terminal_id("replacement item"), id);
        assert_eq!(tools.usage(), (40, 9));
    }
}
