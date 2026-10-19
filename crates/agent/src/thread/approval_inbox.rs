//! Session-scoped approval transitions. GPUI effects and tool execution stay in the coordinator.
use super::permissions::{PermissionCard, PermissionGate, ProviderRequests};
use super::{bound_call::BoundToolCall, session_grants::SessionGrants};
use nocterm_ai::model::auth::ProviderPermission;
use nocterm_ai::model::ids::PermissionOptionId;
use nocterm_ai::{ApprovalPolicy, ApprovalSettings, BridgeCall, TerminalCall};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ApprovalToken(u64);

struct PendingTool {
    token: ApprovalToken,
    call: BoundToolCall,
}

pub(crate) struct ToolApprovalCard {
    token: ApprovalToken,
    call: TerminalCall,
}
impl ToolApprovalCard {
    pub(crate) fn token(&self) -> ApprovalToken {
        self.token
    }
    pub(crate) fn call(&self) -> &TerminalCall {
        &self.call
    }
}

/// Authority is resolved again at intake and at the card action, never cached in a card.
pub(super) struct ToolGate {
    available: bool,
    registration: Option<u64>,
    attached: bool,
    unsafe_input: bool,
    expected: Option<nocterm_workspace::ExpectedSession>,
}
impl ToolGate {
    pub(super) fn from_context(
        call: &TerminalCall,
        available: bool,
        registration: Option<u64>,
        resolved: &[(
            String,
            nocterm_workspace::TerminalEntry,
            nocterm_ai::context::TerminalDescriptor,
        )],
        offline: &[(
            nocterm_ai::context::ServerDescriptor,
            nocterm_workspace::ConnectionSummary,
        )],
        cx: &gpui_kit::App,
    ) -> Self {
        let attached = match (call.terminal_id(), call.server_id()) {
            (Some(id), _) => resolved.iter().any(|(candidate, _, _)| candidate == id),
            (None, Some(id)) => offline.iter().any(|(server, _)| server.server_id == id),
            (None, None) => true,
        };
        let info = resolved
            .iter()
            .find(|(id, _, _)| Some(id.as_str()) == call.terminal_id())
            .and_then(|(_, entry, _)| entry.capabilities.state().info(cx));
        let unsafe_input = matches!(call, TerminalCall::SendInput(_))
            && info.as_ref().is_none_or(|info| {
                info.at_prompt != Some(true) || info.dirty_input || info.alt_screen
            });
        Self::new(
            available,
            registration,
            attached,
            unsafe_input,
            info.map(|info| info.expected),
        )
    }
    pub(super) fn expected(&self) -> Option<nocterm_workspace::ExpectedSession> {
        self.expected
    }

    pub(super) fn new(
        available: bool,
        registration: Option<u64>,
        attached: bool,
        unsafe_input: bool,
        expected: Option<nocterm_workspace::ExpectedSession>,
    ) -> Self {
        Self {
            available,
            registration,
            attached,
            unsafe_input,
            expected,
        }
    }
    fn accepts(&self, call: &BridgeCall) -> bool {
        self.available && self.registration == Some(call.registration_id)
    }
}

pub(super) enum ToolRejection {
    Unavailable,
    Denied,
    Detached,
}
pub(super) enum ToolDecision {
    Respond(BridgeCall, ToolRejection),
    Queue,
    Execute(BoundToolCall),
    Unchanged,
}

#[derive(Default)]
pub(super) struct ApprovalInbox {
    providers: ProviderRequests,
    tools: Vec<PendingTool>,
    generation: u64,
    grants: SessionGrants,
}
impl ApprovalInbox {
    pub(super) fn receive_provider(
        &mut self,
        request: ProviderPermission,
        respond: nocterm_ai::PermissionResponder,
        gate: &PermissionGate,
        policy: ApprovalPolicy,
    ) -> bool {
        let token = ApprovalToken(self.generation.wrapping_add(1));
        if self.providers.receive(
            request,
            respond,
            token,
            gate,
            policy == ApprovalPolicy::Allow,
        ) {
            self.generation = token.0;
            true
        } else {
            false
        }
    }
    pub(super) fn apply_provider_policy(
        &mut self,
        gate: &PermissionGate,
        policy: ApprovalPolicy,
    ) -> bool {
        self.providers
            .apply_policy(gate, policy == ApprovalPolicy::Allow)
    }
    pub(super) fn choose_provider(
        &mut self,
        token: ApprovalToken,
        choice: Option<PermissionOptionId>,
        gate: &PermissionGate,
    ) -> bool {
        self.providers.choose(token, choice, gate)
    }
    pub(super) fn receive_tool(
        &mut self,
        call: BoundToolCall,
        gate: &ToolGate,
        policy: &ApprovalSettings,
    ) -> ToolDecision {
        if !gate.accepts(call.call()) {
            return ToolDecision::Respond(call.into_call(), ToolRejection::Unavailable);
        }
        self.grants.observe(&call.call().call, gate.expected);
        if self.grants.requires_approval(&call.call().call, policy) || gate.unsafe_input {
            self.generation = self.generation.wrapping_add(1);
            self.tools.push(PendingTool {
                token: ApprovalToken(self.generation),
                call,
            });
            ToolDecision::Queue
        } else {
            ToolDecision::Execute(call)
        }
    }
    pub(super) fn choose_tool(
        &mut self,
        token: ApprovalToken,
        allow: bool,
        grant: bool,
        gate: &ToolGate,
    ) -> ToolDecision {
        let Some(index) = self.tools.iter().position(|pending| pending.token == token) else {
            return ToolDecision::Unchanged;
        };
        let call = self.tools.remove(index).call;
        if !allow {
            return ToolDecision::Respond(call.into_call(), ToolRejection::Denied);
        }
        if !gate.accepts(call.call()) || !gate.attached || call.expected() != gate.expected {
            return ToolDecision::Respond(call.into_call(), ToolRejection::Detached);
        }
        self.grants.observe(&call.call().call, gate.expected);
        if grant
            && !gate.unsafe_input
            && let Some(id) = call.call().call.target()
        {
            self.grants.grant(id, call.call().call.capability());
        }
        ToolDecision::Execute(call)
    }
    pub(super) fn tool(&self, token: ApprovalToken) -> Option<&TerminalCall> {
        self.tools
            .iter()
            .find(|pending| pending.token == token)
            .map(|pending| &pending.call.call().call)
    }
    pub(super) fn cancel_pending(&mut self) -> Vec<BridgeCall> {
        self.providers.clear();
        std::mem::take(&mut self.tools)
            .into_iter()
            .map(|pending| pending.call.into_call())
            .collect()
    }
    pub(super) fn revoke_scope(&mut self) {
        self.grants.clear();
    }
    pub(super) fn end_session(&mut self) -> Vec<BridgeCall> {
        self.revoke_scope();
        self.cancel_pending()
    }
    pub(super) fn pending(&self) -> bool {
        self.providers.len() != 0 || !self.tools.is_empty()
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn permission_cards(&self) -> Vec<PermissionCard> {
        self.providers.cards()
    }
    pub(super) fn tool_cards(&self) -> Vec<ToolApprovalCard> {
        self.tools
            .iter()
            .map(|pending| ToolApprovalCard {
                token: pending.token,
                call: pending.call.call().call.clone(),
            })
            .collect()
    }
}
