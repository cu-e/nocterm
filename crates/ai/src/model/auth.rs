use super::{
    extensions::Extensions,
    ids::{AuthMethodId, PermissionOptionId, SessionId},
    tools::ToolPatch,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthMethod {
    pub id: AuthMethodId,
    pub label: String,
    pub description: Option<String>,
    pub flow: AuthFlow,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AuthFlow {
    AgentManaged,
    TerminalSetup(TerminalSetup),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalSetup {
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderPermission {
    pub session: SessionId,
    pub tool: ToolPatch,
    pub choices: Vec<PermissionChoice>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionChoice {
    pub id: PermissionOptionId,
    pub label: String,
    pub effect: PermissionEffect,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
/// Exactly one neutral reply; abandonment cancels, including a closed UI owner.
#[derive(Debug)]
pub struct PermissionResponder(Option<futures::channel::oneshot::Sender<PermissionOutcome>>);
impl PermissionResponder {
    pub fn channel() -> (Self, futures::channel::oneshot::Receiver<PermissionOutcome>) {
        let (sender, receiver) = futures::channel::oneshot::channel();
        (Self(Some(sender)), receiver)
    }
    pub fn respond(mut self, outcome: PermissionOutcome) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(outcome);
        }
    }
}
impl Drop for PermissionResponder {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(PermissionOutcome::Cancelled);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abandonment_cancels_and_selection_is_delivered_once() {
        let (responder, answer) = PermissionResponder::channel();
        drop(responder);
        assert_eq!(
            futures::executor::block_on(answer).unwrap(),
            PermissionOutcome::Cancelled
        );
        let (responder, answer) = PermissionResponder::channel();
        let selected = PermissionOutcome::Selected(PermissionOptionId::new("001"));
        responder.respond(selected.clone());
        assert_eq!(futures::executor::block_on(answer).unwrap(), selected);
    }
}
