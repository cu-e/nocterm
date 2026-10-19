use super::ids::{AuthMethodId, PermissionOptionId, SessionId};
use super::tools::ToolPatch;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct AuthMethod {
    pub id: AuthMethodId,
    pub label: String,
    pub description: Option<String>,
    pub flow: AuthFlow,
}
impl AuthMethod {
    pub fn agent(id: impl Into<AuthMethodId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            flow: AuthFlow::AgentManaged,
        }
    }
    pub fn terminal(
        id: impl Into<AuthMethodId>,
        label: impl Into<String>,
        setup: TerminalSetup,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            flow: AuthFlow::TerminalSetup(setup),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuthFlow {
    AgentManaged,
    TerminalSetup(TerminalSetup),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalSetup {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// A tool call the agent asks the user to authorize.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderPermission {
    pub session: SessionId,
    pub tool: ToolPatch,
    pub choices: Vec<PermissionChoice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionChoice {
    pub id: PermissionOptionId,
    pub label: String,
    pub effect: PermissionEffect,
}
impl PermissionChoice {
    pub fn new(
        id: impl Into<PermissionOptionId>,
        label: impl Into<String>,
        effect: PermissionEffect,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            effect,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionEffect {
    AllowOnce,
    AllowRemembered,
    DenyOnce,
    DenyRemembered,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionOutcome {
    Selected(PermissionOptionId),
    Cancelled,
}
