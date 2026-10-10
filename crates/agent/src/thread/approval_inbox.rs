//! Session-scoped approval transitions. GPUI effects and tool execution stay in the coordinator.
use super::permissions::{PermissionCard, PermissionGate, ProviderRequests};
use nocterm_ai::{
    ApprovalPolicy, ApprovalSettings, BridgeCall, TerminalCall, acp, approval::ApprovalGrants,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ApprovalToken(u64);

struct PendingTool {
    token: ApprovalToken,
    call: BridgeCall,
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
}
impl ToolGate {
    pub(super) fn new(
        available: bool,
        registration: Option<u64>,
        attached: bool,
        unsafe_input: bool,
    ) -> Self {
        Self {
            available,
            registration,
            attached,
            unsafe_input,
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
    Execute(BridgeCall),
    Unchanged,
}

#[derive(Default)]
pub(super) struct ApprovalInbox {
    providers: ProviderRequests,
    tools: Vec<PendingTool>,
    generation: u64,
    grants: ApprovalGrants,
}
impl ApprovalInbox {
    pub(super) fn receive_provider(
        &mut self,
        request: acp::RequestPermissionRequest,
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
        choice: Option<acp::PermissionOptionId>,
        gate: &PermissionGate,
    ) -> bool {
        self.providers.choose(token, choice, gate)
    }
    pub(super) fn receive_tool(
        &mut self,
        call: BridgeCall,
        gate: &ToolGate,
        policy: &ApprovalSettings,
    ) -> ToolDecision {
        if !gate.accepts(&call) {
            return ToolDecision::Respond(call, ToolRejection::Unavailable);
        }
        if self.grants.requires_approval(&call.call, policy) || gate.unsafe_input {
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
            return ToolDecision::Respond(call, ToolRejection::Denied);
        }
        if !gate.accepts(&call) || !gate.attached {
            return ToolDecision::Respond(call, ToolRejection::Detached);
        }
        if grant
            && !gate.unsafe_input
            && let Some(id) = call.call.target()
        {
            self.grants.grant(id, call.call.capability());
        }
        ToolDecision::Execute(call)
    }
    pub(super) fn tool(&self, token: ApprovalToken) -> Option<&TerminalCall> {
        self.tools
            .iter()
            .find(|pending| pending.token == token)
            .map(|pending| &pending.call.call)
    }
    pub(super) fn cancel_pending(&mut self) -> Vec<BridgeCall> {
        self.providers.clear();
        std::mem::take(&mut self.tools)
            .into_iter()
            .map(|pending| pending.call)
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
                call: pending.call.call.clone(),
            })
            .collect()
    }
}
